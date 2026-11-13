//! LRCLIB (first) and Megalobiz (second) search, match, merge, and dedup.
//!
//! Candidates stay in memory. Confirm uses the saved body and does not call HTTP.
//! Lyric bodies are never written to logs or permanent candidate files.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;
use tracing::debug;

use super::lyrics::{LyricsCandidate, LyricsProviderKind, LyricsSearchOutcome, LyricsSearchQuery};
use super::lyrics_http::{LyricsHttp, LyricsHttpError, LyricsHttpRequest, require_success};
use crate::player::metadata::LyricsContent;

const LRCLIB_SEARCH_URL: &str = "https://lrclib.net/api/search";
const MEGALOBIZ_SEARCH_URL: &str = "https://api.megalobiz.com/api/public/v1/search";
const MEGALOBIZ_LRC_URL: &str = "https://api.megalobiz.com/api/public/v1/lrc";
const KEEP_DURATION_MS: u64 = 15_000;
const MEGALOBIZ_LOW_SCORE: f64 = 0.7;
const MEGALOBIZ_FETCH_LIMIT: usize = 8;
const MEGALOBIZ_SEARCH_LIMIT: u32 = 20;

/// Remote lyrics search used in production. HTTP happens only when `search` is called.
#[derive(Debug)]
pub(crate) struct RemoteLyricsSource {
    http: LyricsHttp,
    lrclib_search_url: String,
    megalobiz_search_url: String,
    megalobiz_lrc_url: String,
}

impl RemoteLyricsSource {
    pub(crate) fn production() -> Self {
        Self {
            http: LyricsHttp::production(),
            lrclib_search_url: LRCLIB_SEARCH_URL.to_owned(),
            megalobiz_search_url: MEGALOBIZ_SEARCH_URL.to_owned(),
            megalobiz_lrc_url: MEGALOBIZ_LRC_URL.to_owned(),
        }
    }

    #[cfg(test)]
    fn with_http(http: LyricsHttp) -> Self {
        Self {
            http,
            lrclib_search_url: LRCLIB_SEARCH_URL.to_owned(),
            megalobiz_search_url: MEGALOBIZ_SEARCH_URL.to_owned(),
            megalobiz_lrc_url: MEGALOBIZ_LRC_URL.to_owned(),
        }
    }

    pub(crate) async fn search(&self, query: LyricsSearchQuery) -> LyricsSearchOutcome {
        let query = PreparedQuery::from_search(query);
        let (lrclib, megalobiz) = (
            self.search_lrclib(&query).await,
            self.search_megalobiz(&query).await,
        );
        merge_source_results(lrclib, megalobiz, &query)
    }

    async fn search_lrclib(
        &self,
        query: &PreparedQuery,
    ) -> Result<Vec<ScoredCandidate>, LyricsHttpError> {
        let Some(title) = query.title.as_deref() else {
            return Ok(Vec::new());
        };
        let mut request =
            LyricsHttpRequest::new(&self.lrclib_search_url).with_query("track_name", title);
        if let Some(artist) = query.artist.as_deref() {
            request = request.with_query("artist_name", artist);
        }
        if let Some(album) = query.album.as_deref() {
            request = request.with_query("album_name", album);
        }
        debug!(provider = "lrclib", "searching lyrics");
        let body = require_success(self.http.get(request).await?)?;
        let records: Vec<LrclibTrack> =
            serde_json::from_slice(&body).map_err(|_| LyricsHttpError::InvalidResponse)?;
        // Drop the raw JSON body immediately; only keep selected metadata + lyric text in candidates.
        drop(body);
        Ok(records
            .into_iter()
            .filter_map(|record| self.lrclib_candidate(record, query))
            .collect())
    }

    fn lrclib_candidate(
        &self,
        record: LrclibTrack,
        query: &PreparedQuery,
    ) -> Option<ScoredCandidate> {
        let title = record.track_name.unwrap_or_default();
        let artist = record.artist_name.unwrap_or_default();
        let duration_ms = seconds_to_ms(record.duration);
        let instrumental_flag = record.instrumental.unwrap_or(false);
        let marked_instrumental = instrumental_flag || looks_like_instrumental(&title);
        if should_drop_instrumental(query, marked_instrumental) {
            return None;
        }
        if !query.titles_compatible(&title) {
            return None;
        }
        if !query.artists_compatible(&artist) {
            return None;
        }
        if !query.duration_acceptable(duration_ms) {
            return None;
        }
        let (raw_text, synced) = match (
            lyric_text(record.synced_lyrics),
            lyric_text(record.plain_lyrics),
        ) {
            (Some(synced), _) => (synced, true),
            (None, Some(plain)) => (plain, false),
            (None, None) if marked_instrumental && query.target_is_instrumental => {
                (String::new(), false)
            }
            (None, None) => return None,
        };
        let body = if raw_text.is_empty() {
            LyricsContent::Plain {
                text: String::new(),
            }
        } else {
            LyricsContent::from_text(raw_text.clone())
        };
        Some(score_candidate(
            query,
            LyricsCandidate {
                id: super::lyrics::LyricsCandidateId::from_raw(0),
                provider: LyricsProviderKind::Lrclib,
                title,
                artist,
                duration_ms,
                synced,
                instrumental: Some(marked_instrumental),
                body,
                source_id: Some(record.id.to_string()),
                raw_text: Some(raw_text),
            },
        ))
    }

    async fn search_megalobiz(
        &self,
        query: &PreparedQuery,
    ) -> Result<Vec<ScoredCandidate>, LyricsHttpError> {
        let Some(title) = query.title.as_deref() else {
            return Ok(Vec::new());
        };
        let mut items = Vec::new();
        let mut last_err = None;
        match self
            .megalobiz_search_page(title, query.artist.as_deref(), query)
            .await
        {
            Ok(page) => items.extend(page),
            Err(err) => last_err = Some(err),
        }
        if let Some(alias) = query.title_alias_for_search() {
            match self
                .megalobiz_search_page(&alias, query.artist.as_deref(), query)
                .await
            {
                Ok(extra) => items.extend(extra),
                Err(err) => {
                    debug!(provider = "megalobiz", %err, "alias search failed");
                    if last_err.is_none() {
                        last_err = Some(err);
                    }
                }
            }
        }
        if items.is_empty()
            && let Some(err) = last_err
        {
            return Err(err);
        }
        items.sort_by_key(|item| item.id);
        items.dedup_by(|left, right| left.id == right.id);

        let mut kept = items
            .into_iter()
            .filter(|item| self.keep_megalobiz_item(item, query))
            .collect::<Vec<_>>();
        kept.sort_by(|left, right| {
            score_megalobiz(query, left)
                .partial_cmp(&score_megalobiz(query, right))
                .unwrap_or(std::cmp::Ordering::Equal)
                .reverse()
        });
        kept.truncate(MEGALOBIZ_FETCH_LIMIT);

        let mut candidates = Vec::new();
        let mut fetch_err = None;
        for item in kept {
            match self.fetch_megalobiz_lrc(item.id).await {
                Ok(raw_text) => {
                    if let Some(candidate) = self.megalobiz_candidate(item, raw_text, query) {
                        candidates.push(candidate);
                    }
                }
                Err(err) => {
                    debug!(provider = "megalobiz", id = item.id, %err, "skip lrc fetch");
                    if fetch_err.is_none() {
                        fetch_err = Some(err);
                    }
                }
            }
        }
        if candidates.is_empty()
            && let Some(err) = fetch_err
        {
            return Err(err);
        }
        Ok(candidates)
    }

    async fn megalobiz_search_page(
        &self,
        title: &str,
        artist: Option<&str>,
        query: &PreparedQuery,
    ) -> Result<Vec<MegalobizItem>, LyricsHttpError> {
        let mut request = LyricsHttpRequest::new(&self.megalobiz_search_url)
            .with_query("title", title)
            .with_query("limit", MEGALOBIZ_SEARCH_LIMIT.to_string());
        if let Some(artist) = artist {
            request = request.with_query("artist", artist);
        }
        if let Some(seconds) = query.duration_secs() {
            request = request.with_query("duration", format!("{seconds:.0}"));
        }
        debug!(provider = "megalobiz", "searching lyrics");
        let body = require_success(self.http.get(request).await?)?;
        let items = parse_megalobiz_search(&body)?;
        drop(body);
        Ok(items)
    }

    fn keep_megalobiz_item(&self, item: &MegalobizItem, query: &PreparedQuery) -> bool {
        if looks_like_instrumental(&item.title) && should_drop_instrumental(query, true) {
            return false;
        }
        if !query.titles_compatible(&item.title) {
            return false;
        }
        let artist = item.artist.as_deref().unwrap_or("");
        if artist.trim().is_empty() {
            let low_score = item.score.unwrap_or(0.0) < MEGALOBIZ_LOW_SCORE;
            if low_score || !query.title_is_exact_or_alias(&item.title) {
                return false;
            }
        } else if !query.artists_compatible(artist) {
            return false;
        }
        query.duration_acceptable(seconds_to_ms(item.duration))
    }

    async fn fetch_megalobiz_lrc(&self, id: u64) -> Result<String, LyricsHttpError> {
        let request = LyricsHttpRequest::new(format!("{}/{id}", self.megalobiz_lrc_url))
            .with_query("format", "lrc");
        let body = require_success(self.http.get(request).await?)?;
        if body.is_empty() {
            return Err(LyricsHttpError::InvalidResponse);
        }
        String::from_utf8(body).map_err(|_| LyricsHttpError::InvalidResponse)
    }

    fn megalobiz_candidate(
        &self,
        item: MegalobizItem,
        raw_text: String,
        query: &PreparedQuery,
    ) -> Option<ScoredCandidate> {
        let title = item.title;
        let artist = item.artist.unwrap_or_default();
        let duration_ms = seconds_to_ms(item.duration);
        let marked_instrumental = looks_like_instrumental(&title);
        if raw_text.trim().is_empty() {
            return None;
        }
        let body = LyricsContent::from_text(raw_text.clone());
        let synced = matches!(body, LyricsContent::Timed { .. });
        Some(score_candidate(
            query,
            LyricsCandidate {
                id: super::lyrics::LyricsCandidateId::from_raw(0),
                provider: LyricsProviderKind::Megalobiz,
                title,
                artist,
                duration_ms,
                synced,
                instrumental: Some(marked_instrumental),
                body,
                source_id: Some(item.id.to_string()),
                raw_text: Some(raw_text),
            },
        ))
    }
}

fn merge_source_results(
    lrclib: Result<Vec<ScoredCandidate>, LyricsHttpError>,
    megalobiz: Result<Vec<ScoredCandidate>, LyricsHttpError>,
    query: &PreparedQuery,
) -> LyricsSearchOutcome {
    let mut candidates = Vec::new();
    let mut errors = Vec::new();
    match lrclib {
        Ok(items) => candidates.extend(items),
        Err(err) => errors.push(err),
    }
    match megalobiz {
        Ok(items) => candidates.extend(items),
        Err(err) => errors.push(err),
    }
    if candidates.is_empty() {
        return if errors.is_empty() {
            LyricsSearchOutcome::from_result(Ok(Vec::new()))
        } else {
            LyricsSearchOutcome::from_result(Err(errors[0].user_message()))
        };
    }
    LyricsSearchOutcome::from_result(Ok(dedup_and_sort(candidates, query)))
}

fn dedup_and_sort(
    mut candidates: Vec<ScoredCandidate>,
    query: &PreparedQuery,
) -> Vec<LyricsCandidate> {
    candidates.sort_by(|left, right| right.cmp_rank(left));
    let mut kept: Vec<ScoredCandidate> = Vec::new();
    for candidate in candidates {
        let duplicate = kept.iter().any(|existing| {
            same_source(existing, &candidate)
                || (same_identity(existing, &candidate, query) && same_body(existing, &candidate))
                || same_body(existing, &candidate)
        });
        if !duplicate {
            kept.push(candidate);
        }
    }
    kept.into_iter().map(|candidate| candidate.inner).collect()
}

fn same_source(left: &ScoredCandidate, right: &ScoredCandidate) -> bool {
    left.inner.provider == right.inner.provider
        && left.inner.source_id.is_some()
        && left.inner.source_id == right.inner.source_id
}

fn same_identity(left: &ScoredCandidate, right: &ScoredCandidate, _query: &PreparedQuery) -> bool {
    normalize_person_or_title(&left.inner.title) == normalize_person_or_title(&right.inner.title)
        && durations_close(left.inner.duration_ms, right.inner.duration_ms, 1_000)
}

fn same_body(left: &ScoredCandidate, right: &ScoredCandidate) -> bool {
    normalize_body(left.raw_body()) == normalize_body(right.raw_body())
        && !normalize_body(left.raw_body()).is_empty()
}

#[derive(Debug, Clone)]
struct ScoredCandidate {
    inner: LyricsCandidate,
    synced: bool,
    duration_delta: Option<u64>,
    title_exact: bool,
    provider_order: u8,
}

impl ScoredCandidate {
    fn raw_body(&self) -> &str {
        self.inner.raw_text.as_deref().unwrap_or("")
    }

    fn cmp_rank(&self, other: &Self) -> std::cmp::Ordering {
        self.synced
            .cmp(&other.synced)
            .then_with(|| match (self.duration_delta, other.duration_delta) {
                (Some(left), Some(right)) => right.cmp(&left),
                (Some(_), None) => std::cmp::Ordering::Greater,
                (None, Some(_)) => std::cmp::Ordering::Less,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| self.title_exact.cmp(&other.title_exact))
            .then_with(|| other.provider_order.cmp(&self.provider_order))
    }
}

fn score_candidate(query: &PreparedQuery, inner: LyricsCandidate) -> ScoredCandidate {
    let synced = inner.synced || matches!(inner.body, LyricsContent::Timed { .. });
    ScoredCandidate {
        duration_delta: query
            .duration_ms
            .zip(inner.duration_ms)
            .map(|(expected, actual)| expected.abs_diff(actual)),
        title_exact: query.title_is_exact_or_alias(&inner.title),
        provider_order: match inner.provider {
            LyricsProviderKind::Lrclib => 0,
            LyricsProviderKind::Megalobiz => 1,
            LyricsProviderKind::Fixture => 2,
        },
        synced,
        inner,
    }
}

fn score_megalobiz(query: &PreparedQuery, item: &MegalobizItem) -> f64 {
    let duration = seconds_to_ms(item.duration)
        .and_then(|actual| query.duration_ms.map(|expected| expected.abs_diff(actual)))
        .map(|delta| 1.0 - (delta as f64 / KEEP_DURATION_MS as f64).min(1.0))
        .unwrap_or(0.5);
    let title = if query.title_is_exact_or_alias(&item.title) {
        1.0
    } else {
        0.4
    };
    item.score.unwrap_or(0.5) * 0.4 + duration * 0.4 + title * 0.2
}

#[derive(Debug, Clone)]
struct PreparedQuery {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    duration_ms: Option<u64>,
    title_aliases: Vec<String>,
    artist_aliases: Vec<String>,
    target_is_instrumental: bool,
}

impl PreparedQuery {
    fn from_search(query: LyricsSearchQuery) -> Self {
        let title = nonempty_text(query.title.as_deref()).or_else(|| title_from_path(&query.path));
        let artist = nonempty_text(query.artist.as_deref());
        let album = nonempty_text(query.album.as_deref());
        let title_aliases = title.as_deref().map(title_aliases).unwrap_or_default();
        let artist_aliases = artist.as_deref().map(artist_aliases).unwrap_or_default();
        let target_is_instrumental = title.as_deref().is_some_and(looks_like_instrumental);
        Self {
            title,
            artist,
            album,
            duration_ms: query.duration_ms,
            title_aliases,
            artist_aliases,
            target_is_instrumental,
        }
    }

    fn duration_secs(&self) -> Option<f64> {
        self.duration_ms.map(|ms| ms as f64 / 1000.0)
    }

    fn duration_acceptable(&self, candidate: Option<u64>) -> bool {
        match (self.duration_ms, candidate) {
            (Some(expected), Some(actual)) => expected.abs_diff(actual) <= KEEP_DURATION_MS,
            _ => true,
        }
    }

    fn title_alias_for_search(&self) -> Option<String> {
        let original = normalize_person_or_title(self.title.as_deref()?);
        self.title_aliases
            .iter()
            .find(|alias| normalize_person_or_title(alias) != original)
            .cloned()
    }

    fn title_is_exact_or_alias(&self, candidate: &str) -> bool {
        let candidate_norm = normalize_person_or_title(candidate);
        if let Some(title) = &self.title
            && normalize_person_or_title(title) == candidate_norm
        {
            return true;
        }
        self.title_aliases
            .iter()
            .any(|alias| normalize_person_or_title(alias) == candidate_norm)
    }

    fn titles_compatible(&self, candidate: &str) -> bool {
        let Some(title) = self.title.as_deref() else {
            return true;
        };
        titles_compatible(title, candidate, &self.title_aliases)
    }

    fn artists_compatible(&self, candidate: &str) -> bool {
        let Some(artist) = self.artist.as_deref() else {
            return true;
        };
        artists_compatible(artist, candidate, &self.artist_aliases)
    }
}

fn title_from_path(path: &str) -> Option<String> {
    let name = std::path::Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())?
        .trim();
    nonempty_text(Some(name))
}

fn titles_compatible(query: &str, candidate: &str, aliases: &[String]) -> bool {
    let query_norm = normalize_person_or_title(&strip_title_decoration(query));
    let candidate_norm = normalize_person_or_title(&strip_title_decoration(candidate));
    if query_norm.is_empty() || candidate_norm.is_empty() {
        return false;
    }
    if query_norm == candidate_norm {
        return true;
    }
    if aliases.iter().any(|alias| {
        let alias_norm = normalize_person_or_title(alias);
        !alias_norm.is_empty() && alias_norm != query_norm && alias_norm == candidate_norm
    }) {
        return true;
    }
    // Allow a parenthetical / feat suffix, but not extra significant words
    // ("Rubia sol morena luna" must not match a query of "Rubia").
    let query_tokens = tokens(&query_norm);
    let candidate_tokens = tokens(&candidate_norm);
    if query_tokens.is_empty() || candidate_tokens.is_empty() {
        return false;
    }
    if candidate_tokens.starts_with(&query_tokens)
        && candidate_tokens
            .iter()
            .skip(query_tokens.len())
            .all(|token| is_filler_title_token(token))
    {
        return true;
    }
    if query_tokens.starts_with(&candidate_tokens)
        && query_tokens
            .iter()
            .skip(candidate_tokens.len())
            .all(|token| is_filler_title_token(token))
    {
        return true;
    }
    aliases.iter().any(|alias| {
        let alias_tokens = tokens(&normalize_person_or_title(alias));
        candidate_tokens == alias_tokens
    })
}

fn artists_compatible(query: &str, candidate: &str, aliases: &[String]) -> bool {
    let query_parts = artist_parts(query, aliases);
    let candidate_parts = artist_parts(candidate, aliases);
    if query_parts.is_empty() || candidate_parts.is_empty() {
        return false;
    }
    query_parts.iter().any(|query_part| {
        candidate_parts
            .iter()
            .any(|candidate_part| artist_parts_overlap(query_part, candidate_part))
    })
}

fn artist_parts_overlap(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    let left_tokens = tokens(left);
    let right_tokens = tokens(right);
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return false;
    }
    let (shorter, longer) = if left_tokens.len() <= right_tokens.len() {
        (&left_tokens, &right_tokens)
    } else {
        (&right_tokens, &left_tokens)
    };
    // "zhou shen" matches "zhou shen hoyomix", but "mix" must not match an unrelated artist.
    shorter.iter().all(|token| longer.contains(token)) && shorter.len() >= 2
        || shorter.len() == 1 && longer.iter().any(|token| token == &shorter[0])
}

fn artist_parts(value: &str, aliases: &[String]) -> Vec<String> {
    let mut parts = split_artists(value);
    for alias in aliases {
        parts.extend(split_artists(alias));
    }
    for part in parts.clone() {
        parts.extend(aliases_for_artist(&part));
    }
    parts.sort();
    parts.dedup();
    parts
}

fn split_artists(value: &str) -> Vec<String> {
    let replaced = value
        .replace(" feat. ", "/")
        .replace(" ft. ", "/")
        .replace(" featuring ", "/")
        .replace(" 和 ", "/")
        .replace(['&', '•', ';', ',', '、'], "/");
    replaced
        .split('/')
        .filter_map(|part| nonempty_text(Some(part)).map(|text| normalize_person_or_title(&text)))
        .filter(|part| !part.is_empty())
        .collect()
}

fn title_aliases(title: &str) -> Vec<String> {
    let mut aliases = vec![title.to_owned()];
    let norm = normalize_person_or_title(title);
    for (left, right) in TITLE_ALIASES {
        if norm == normalize_person_or_title(left) {
            aliases.push((*right).to_owned());
        } else if norm == normalize_person_or_title(right) {
            aliases.push((*left).to_owned());
        }
    }
    aliases
}

fn artist_aliases(artist: &str) -> Vec<String> {
    let mut aliases = vec![artist.to_owned()];
    for part in split_artists(artist) {
        aliases.extend(aliases_for_artist(&part));
    }
    aliases
}

fn aliases_for_artist(part: &str) -> Vec<String> {
    let norm = normalize_person_or_title(part);
    let mut aliases = Vec::new();
    for (left, right) in ARTIST_ALIASES {
        if norm == normalize_person_or_title(left) || norm == normalize_person_or_title(right) {
            aliases.push((*left).to_owned());
            aliases.push((*right).to_owned());
        }
    }
    aliases
}

const TITLE_ALIASES: &[(&str, &str)] = &[("轻涟", "La vaguelette"), ("野火", "Wildfire")];

const ARTIST_ALIASES: &[(&str, &str)] = &[
    ("周深", "Zhou Shen"),
    ("HOYO-MiX", "hoyomix"),
    ("HOYO-MiX", "HOYO-MIX"),
];

fn looks_like_instrumental(title: &str) -> bool {
    let norm = normalize_person_or_title(title);
    const MARKERS: &[&str] = &[
        "instrumental",
        "karaoke",
        "off vocal",
        "offvocal",
        "inst",
        "伴奏",
        "纯音乐",
        "instrumental version",
    ];
    MARKERS.iter().any(|marker| {
        let marker_norm = normalize_person_or_title(marker);
        norm == marker_norm
            || norm.contains(&format!(" {marker_norm} "))
            || norm.ends_with(&format!(" {marker_norm}"))
            || norm.starts_with(&format!("{marker_norm} "))
            || (!marker.is_ascii() && title.contains(marker))
    })
}

fn should_drop_instrumental(query: &PreparedQuery, candidate_instrumental: bool) -> bool {
    candidate_instrumental && !query.target_is_instrumental
}

fn normalize_person_or_title(value: &str) -> String {
    let lower = value.trim().to_lowercase();
    let stripped = lower
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == ' ' || !ch.is_ascii() {
                ch
            } else {
                ' '
            }
        })
        .collect::<String>();
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_title_decoration(title: &str) -> String {
    let mut text = title.trim().to_owned();
    if let Some(start) = text.find('(')
        && let Some(end) = text[start..].find(')')
    {
        let inner = &text[start + 1..start + end];
        if looks_like_instrumental(inner)
            || is_filler_title_token(&normalize_person_or_title(inner))
            || inner.len() > 2
        {
            text.replace_range(start..start + end + 1, " ");
        }
    }
    text
}

fn tokens(normalized: &str) -> Vec<String> {
    normalized
        .split_whitespace()
        .map(ToOwned::to_owned)
        .collect()
}

fn is_filler_title_token(token: &str) -> bool {
    matches!(
        token,
        "feat"
            | "ft"
            | "featuring"
            | "official"
            | "audio"
            | "version"
            | "ver"
            | "theme"
            | "ost"
            | "movie"
            | "tv"
    )
}

fn lyric_text(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}

fn nonempty_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "null")
        .map(ToOwned::to_owned)
}

fn seconds_to_ms(value: Option<f64>) -> Option<u64> {
    let seconds = value?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some((seconds * 1000.0).round() as u64)
}

fn durations_close(left: Option<u64>, right: Option<u64>, tolerance: u64) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.abs_diff(right) <= tolerance,
        _ => false,
    }
}

fn normalize_body(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("[ar:")
                && !line.starts_with("[ti:")
                && !line.starts_with("[al:")
                && !line.starts_with("[by:")
                && !line.starts_with("[offset:")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase()
}

#[derive(Debug, Deserialize)]
struct LrclibTrack {
    id: u64,
    #[serde(rename = "trackName")]
    track_name: Option<String>,
    #[serde(rename = "artistName")]
    artist_name: Option<String>,
    duration: Option<f64>,
    instrumental: Option<bool>,
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MegalobizItem {
    id: u64,
    title: String,
    #[serde(default)]
    artist: Option<String>,
    duration: Option<f64>,
    #[serde(default)]
    score: Option<f64>,
}

fn parse_megalobiz_search(bytes: &[u8]) -> Result<Vec<MegalobizItem>, LyricsHttpError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| LyricsHttpError::InvalidResponse)?;
    let array = extract_array(&value).ok_or(LyricsHttpError::InvalidResponse)?;
    serde_json::from_value(array).map_err(|_| LyricsHttpError::InvalidResponse)
}

fn extract_array(value: &Value) -> Option<Value> {
    if value.is_array() {
        return Some(value.clone());
    }
    for key in ["data", "results", "items"] {
        if let Some(found) = value.get(key) {
            if found.is_array() {
                return Some(found.clone());
            }
            if let Some(nested) = found.get("items").or_else(|| found.get("results"))
                && nested.is_array()
            {
                return Some(nested.clone());
            }
        }
    }
    None
}

pub(crate) fn remote_search_source() -> Arc<RemoteLyricsSource> {
    Arc::new(RemoteLyricsSource::production())
}

#[cfg(test)]
mod tests {
    use super::super::lyrics_http::{LyricsHttpError, LyricsHttpResponse, MockTransport};
    use super::*;
    use crate::player::metadata::LyricsContent;

    fn query(title: &str, artist: &str, duration_ms: u64) -> LyricsSearchQuery {
        LyricsSearchQuery {
            path: format!("/music/{title}.flac"),
            title: Some(title.into()),
            artist: Some(artist.into()),
            album: None,
            duration_ms: Some(duration_ms),
        }
    }

    type LrclibRow<'a> = (
        &'a str,
        &'a str,
        f64,
        bool,
        Option<&'a str>,
        Option<&'a str>,
        u64,
    );

    fn lrclib_json(rows: &[LrclibRow<'_>]) -> String {
        let items: Vec<Value> = rows
            .iter()
            .map(
                |(title, artist, duration, instrumental, synced, plain, id)| {
                    serde_json::json!({
                        "id": id,
                        "trackName": title,
                        "artistName": artist,
                        "albumName": title,
                        "duration": duration,
                        "instrumental": instrumental,
                        "syncedLyrics": synced,
                        "plainLyrics": plain,
                    })
                },
            )
            .collect();
        serde_json::to_string(&items).expect("json")
    }

    fn mega_search_json(rows: &[(&str, &str, f64, f64, u64)]) -> String {
        let items: Vec<Value> = rows
            .iter()
            .map(|(title, artist, duration, score, id)| {
                serde_json::json!({
                    "id": id,
                    "title": title,
                    "artist": artist,
                    "duration": duration,
                    "duration_diff": 0.0,
                    "score": score,
                    "sync": "manual",
                })
            })
            .collect();
        serde_json::to_string(&serde_json::json!({ "data": items })).expect("json")
    }

    fn lrc(line: &str) -> String {
        format!("[00:00.000]{line}\n")
    }

    fn search(source: &RemoteLyricsSource, query: LyricsSearchQuery) -> Vec<LyricsCandidate> {
        match async_global_executor::block_on(source.search(query)) {
            LyricsSearchOutcome::Ready(candidates) => candidates,
            LyricsSearchOutcome::Failed(message) => panic!("search failed: {message}"),
        }
    }

    #[test]
    fn merges_sources_prefers_synced_and_dedups_same_body() {
        let http = MockTransport::new();
        let body = lrc("同一句词");
        http.enqueue_ok(
            "lrclib.net/api/search",
            vec![("track_name".into(), "Rubia".into())],
            lrclib_json(&[(
                "Rubia",
                "Zhou Shen",
                195.0,
                false,
                Some(body.as_str()),
                None,
                11,
            )]),
        );
        http.enqueue_ok(
            "megalobiz.com/api/public/v1/search",
            vec![("title".into(), "Rubia".into())],
            mega_search_json(&[("Rubia", "Zhou Shen", 195.2, 1.0, 22)]),
        );
        http.enqueue_ok(
            "megalobiz.com/api/public/v1/lrc/22",
            vec![("format".into(), "lrc".into())],
            &body,
        );
        let source = RemoteLyricsSource::with_http(LyricsHttp::Mock(http));
        let candidates = search(&source, query("Rubia", "周深", 195_000));
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].provider, LyricsProviderKind::Lrclib);
        assert_eq!(candidates[0].source_id.as_deref(), Some("11"));
        assert_eq!(candidates[0].raw_text.as_deref(), Some(body.as_str()));
        assert!(candidates[0].synced);
    }

    #[test]
    fn filters_instrumental_when_target_is_vocal_but_keeps_when_target_is_instrumental() {
        let http = MockTransport::new();
        http.enqueue_ok(
            "lrclib.net/api/search",
            vec![("track_name".into(), "Rubia".into())],
            lrclib_json(&[
                (
                    "Rubia",
                    "Zhou Shen",
                    195.0,
                    false,
                    Some("[00:00.000]人声\n"),
                    None,
                    1,
                ),
                (
                    "Rubia (Instrumental)",
                    "Zhou Shen",
                    194.0,
                    false,
                    Some("[00:00.000]伴奏\n"),
                    None,
                    2,
                ),
                (
                    "Rubia",
                    "Zhou Shen",
                    195.0,
                    true,
                    Some("[00:00.000]标记伴奏\n"),
                    None,
                    3,
                ),
            ]),
        );
        http.enqueue_ok(
            "megalobiz.com/api/public/v1/search",
            vec![("title".into(), "Rubia".into())],
            mega_search_json(&[]),
        );
        let source = RemoteLyricsSource::with_http(LyricsHttp::Mock(http.clone()));
        let vocal = search(&source, query("Rubia", "Zhou Shen", 195_000));
        assert_eq!(vocal.len(), 1);
        assert_eq!(vocal[0].title, "Rubia");
        assert_eq!(vocal[0].source_id.as_deref(), Some("1"));

        http.enqueue_ok(
            "lrclib.net/api/search",
            vec![("track_name".into(), "Rubia (Instrumental)".into())],
            lrclib_json(&[(
                "Rubia (Instrumental)",
                "Zhou Shen",
                194.0,
                true,
                Some("[00:00.000]伴奏\n"),
                None,
                2,
            )]),
        );
        http.enqueue_ok(
            "megalobiz.com/api/public/v1/search",
            vec![("title".into(), "Rubia (Instrumental)".into())],
            mega_search_json(&[]),
        );
        let inst = search(&source, query("Rubia (Instrumental)", "Zhou Shen", 194_000));
        assert_eq!(inst.len(), 1);
        assert_eq!(inst[0].instrumental, Some(true));
    }

    #[test]
    fn drops_extreme_duration_and_off_topic_same_title() {
        let http = MockTransport::new();
        http.enqueue_ok(
            "lrclib.net/api/search",
            vec![("track_name".into(), "Rubia".into())],
            lrclib_json(&[(
                "Rubia",
                "Zhou Shen",
                3.0,
                false,
                Some("[00:00.000]错时长\n"),
                None,
                7,
            )]),
        );
        http.enqueue_ok(
            "megalobiz.com/api/public/v1/search",
            vec![("title".into(), "Rubia".into())],
            mega_search_json(&[
                (
                    "Rubia sol morena luna",
                    "Caramelos de cianuro",
                    193.0,
                    0.9,
                    99,
                ),
                ("Rubia", "", 195.0, 0.65, 50),
                ("Rubia", "Zhou Shen", 195.4, 1.0, 22),
            ]),
        );
        http.enqueue_ok(
            "megalobiz.com/api/public/v1/lrc/22",
            vec![("format".into(), "lrc".into())],
            lrc("正词"),
        );
        let source = RemoteLyricsSource::with_http(LyricsHttp::Mock(http.clone()));
        let candidates = search(&source, query("Rubia", "Zhou Shen", 195_000));
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].provider, LyricsProviderKind::Megalobiz);
        assert_eq!(candidates[0].source_id.as_deref(), Some("22"));
        let hits = http.hits();
        assert!(
            hits.iter()
                .all(|hit| !hit.contains("/lrc/99") && !hit.contains("/lrc/50"))
        );
    }

    #[test]
    fn chinese_title_alias_can_select_megalobiz_english_title() {
        let http = MockTransport::new();
        http.enqueue_ok(
            "lrclib.net/api/search",
            vec![("track_name".into(), "轻涟".into())],
            lrclib_json(&[]),
        );
        http.enqueue_ok(
            "megalobiz.com/api/public/v1/search",
            vec![("title".into(), "轻涟".into())],
            mega_search_json(&[
                ("La vaguelette", "HOYO-MiX", 149.55, 0.65, 8),
                ("Had I Not Seen the Sun", "HOYO-MiX", 145.0, 0.52, 9),
            ]),
        );
        http.enqueue_ok(
            "megalobiz.com/api/public/v1/search",
            vec![("title".into(), "La vaguelette".into())],
            mega_search_json(&[("La vaguelette", "HOYO-MiX", 149.55, 0.99, 8)]),
        );
        http.enqueue_ok(
            "megalobiz.com/api/public/v1/lrc/8",
            vec![("format".into(), "lrc".into())],
            lrc("轻涟词"),
        );
        let source = RemoteLyricsSource::with_http(LyricsHttp::Mock(http.clone()));
        let candidates = search(&source, query("轻涟", "HOYO-MiX", 150_000));
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].title, "La vaguelette");
        assert!(http.hits().iter().all(|hit| !hit.contains("/lrc/9")));
    }

    #[test]
    fn rate_limit_and_timeout_are_visible_and_write_nothing() {
        let http = MockTransport::new();
        http.enqueue(
            "lrclib.net/api/search",
            Vec::new(),
            Err(LyricsHttpError::Timeout),
        );
        http.enqueue(
            "megalobiz.com/api/public/v1/search",
            Vec::new(),
            Ok(LyricsHttpResponse {
                status: 429,
                retry_after: Some(20),
                body: b"limit body should stay hidden".to_vec(),
            }),
        );
        let source = RemoteLyricsSource::with_http(LyricsHttp::Mock(http));
        match async_global_executor::block_on(source.search(query("Rubia", "Zhou Shen", 195_000))) {
            LyricsSearchOutcome::Failed(message) => {
                assert!(
                    message.contains("超时") || message.contains("频繁"),
                    "unexpected message: {message}"
                );
                assert!(!message.contains("limit body"));
            }
            LyricsSearchOutcome::Ready(candidates) => {
                panic!("expected failure, got {} candidates", candidates.len())
            }
        }
    }

    #[test]
    fn one_source_failure_still_returns_the_other() {
        let http = MockTransport::new();
        http.enqueue(
            "lrclib.net/api/search",
            Vec::new(),
            Err(LyricsHttpError::Network),
        );
        http.enqueue_ok(
            "megalobiz.com/api/public/v1/search",
            vec![("title".into(), "Wildfire".into())],
            mega_search_json(&[("Wildfire", "HOYO-MiX", 218.2, 1.0, 5)]),
        );
        http.enqueue_ok(
            "megalobiz.com/api/public/v1/lrc/5",
            vec![("format".into(), "lrc".into())],
            lrc("野火词"),
        );
        let source = RemoteLyricsSource::with_http(LyricsHttp::Mock(http));
        let candidates = search(&source, query("野火", "HOYO-MiX", 218_000));
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].provider, LyricsProviderKind::Megalobiz);
    }

    #[test]
    fn too_large_response_is_a_visible_failure() {
        let http = MockTransport::new();
        http.enqueue(
            "lrclib.net/api/search",
            Vec::new(),
            Err(LyricsHttpError::TooLarge { limit: 1_048_576 }),
        );
        http.enqueue(
            "megalobiz.com/api/public/v1/search",
            Vec::new(),
            Err(LyricsHttpError::TooLarge { limit: 1_048_576 }),
        );
        let source = RemoteLyricsSource::with_http(LyricsHttp::Mock(http));
        match async_global_executor::block_on(source.search(query("Rubia", "Zhou Shen", 195_000))) {
            LyricsSearchOutcome::Failed(message) => assert!(message.contains("过大")),
            LyricsSearchOutcome::Ready(_) => panic!("expected too-large failure"),
        }
    }

    #[test]
    fn path_stem_is_used_when_tags_are_missing() {
        let http = MockTransport::new();
        http.enqueue_ok(
            "lrclib.net/api/search",
            vec![("track_name".into(), "Rubia".into())],
            lrclib_json(&[(
                "Rubia",
                "Zhou Shen",
                195.0,
                false,
                Some("[00:00.000]词\n"),
                None,
                1,
            )]),
        );
        http.enqueue_ok(
            "megalobiz.com/api/public/v1/search",
            vec![("title".into(), "Rubia".into())],
            mega_search_json(&[]),
        );
        let source = RemoteLyricsSource::with_http(LyricsHttp::Mock(http));
        let candidates = search(
            &source,
            LyricsSearchQuery {
                path: "/music/Rubia.flac".into(),
                title: None,
                artist: None,
                album: None,
                duration_ms: Some(195_000),
            },
        );
        assert_eq!(candidates.len(), 1);
        assert!(matches!(candidates[0].body, LyricsContent::Timed { .. }));
    }

    #[test]
    fn in_memory_body_is_used_for_confirm_without_another_fetch() {
        let http = MockTransport::new();
        http.enqueue_ok(
            "lrclib.net/api/search",
            vec![("track_name".into(), "Rubia".into())],
            lrclib_json(&[(
                "Rubia",
                "Zhou Shen",
                195.0,
                false,
                Some("[00:00.000]内存稿\n"),
                None,
                1,
            )]),
        );
        http.enqueue_ok(
            "megalobiz.com/api/public/v1/search",
            vec![("title".into(), "Rubia".into())],
            mega_search_json(&[]),
        );
        let source = RemoteLyricsSource::with_http(LyricsHttp::Mock(http.clone()));
        let candidates = search(&source, query("Rubia", "Zhou Shen", 195_000));
        let hits_after_search = http.hits().len();
        assert_eq!(candidates[0].text_for_save(), "[00:00.000]内存稿\n");
        assert_eq!(http.hits().len(), hits_after_search);
    }

    #[test]
    #[ignore = "live network smoke; run with --ignored and a working network"]
    fn lrclib_live_search_smoke() {
        let source = RemoteLyricsSource::production();
        let outcome =
            async_global_executor::block_on(source.search(query("Rubia", "Zhou Shen", 195_000)));
        match outcome {
            LyricsSearchOutcome::Ready(candidates) => {
                assert!(
                    candidates
                        .iter()
                        .any(|candidate| candidate.provider == LyricsProviderKind::Lrclib
                            && normalize_person_or_title(&candidate.title) == "rubia"),
                    "expected a Rubia candidate, got {} items (titles omitted to avoid logging lyrics)",
                    candidates.len()
                );
            }
            LyricsSearchOutcome::Failed(message) => {
                panic!("live smoke failed without lyric bodies: {message}");
            }
        }
    }
    #[test]
    fn all_megalobiz_body_failures_are_visible_not_no_results() {
        for err in [
            LyricsHttpError::rate_limited(Some(30)),
            LyricsHttpError::Timeout,
            LyricsHttpError::Network,
        ] {
            let http = MockTransport::new();
            http.enqueue_ok("lrclib.net/api/search", Vec::new(), lrclib_json(&[]));
            http.enqueue_ok(
                "megalobiz.com/api/public/v1/search",
                Vec::new(),
                mega_search_json(&[("Rubia", "Zhou Shen", 195.0, 1.0, 22)]),
            );
            http.enqueue(
                "megalobiz.com/api/public/v1/lrc/22",
                Vec::new(),
                Err(err.clone()),
            );
            let source = RemoteLyricsSource::with_http(LyricsHttp::Mock(http));
            match async_global_executor::block_on(source.search(query(
                "Rubia",
                "Zhou Shen",
                195_000,
            ))) {
                LyricsSearchOutcome::Failed(message) => assert_eq!(message, err.user_message()),
                LyricsSearchOutcome::Ready(_) => panic!("body failure must not become no results"),
            }
        }
    }
    #[test]
    fn one_megalobiz_body_failure_keeps_successful_candidate() {
        let http = MockTransport::new();
        http.enqueue_ok("lrclib.net/api/search", Vec::new(), lrclib_json(&[]));
        http.enqueue_ok(
            "megalobiz.com/api/public/v1/search",
            Vec::new(),
            mega_search_json(&[
                ("Rubia", "Zhou Shen", 195.0, 1.0, 22),
                ("Rubia", "Zhou Shen", 196.0, 1.0, 23),
            ]),
        );
        http.enqueue(
            "megalobiz.com/api/public/v1/lrc/22",
            Vec::new(),
            Err(LyricsHttpError::Timeout),
        );
        http.enqueue_ok(
            "megalobiz.com/api/public/v1/lrc/23",
            Vec::new(),
            lrc("success"),
        );
        let source = RemoteLyricsSource::with_http(LyricsHttp::Mock(http));
        assert_eq!(
            search(&source, query("Rubia", "Zhou Shen", 195_000)).len(),
            1
        );
    }
}
