/// 将毫秒时长格式化为 `m:ss` 或 `h:mm:ss`。
abstract final class DurationFormat {
  static String fromMs(BigInt ms) {
    final total = ms.toInt().clamp(0, 1 << 31);
    return fromMilliseconds(total);
  }

  static String fromMilliseconds(int ms) {
    final totalSeconds = (ms ~/ 1000).clamp(0, 24 * 3600 * 100);
    final hours = totalSeconds ~/ 3600;
    final minutes = (totalSeconds % 3600) ~/ 60;
    final seconds = totalSeconds % 60;
    if (hours > 0) {
      return '$hours:${minutes.toString().padLeft(2, '0')}:${seconds.toString().padLeft(2, '0')}';
    }
    return '$minutes:${seconds.toString().padLeft(2, '0')}';
  }
}
