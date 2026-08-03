import 'dart:io';

import 'package:image/image.dart' as img;

const _logoPath = 'resources/logo.png';
const _appId = 'io.holo.mofu';
const _sizes = [64, 256, 512];

void main() {
  final logoFile = File(_logoPath);
  if (!logoFile.existsSync()) {
    stderr.writeln('logo not found: $_logoPath');
    exit(1);
  }

  final decoded = img.decodeImage(logoFile.readAsBytesSync());
  if (decoded == null) {
    stderr.writeln('failed to decode: $_logoPath');
    exit(1);
  }

  final iconsRoot = Directory('resources/icons/hicolor');
  if (iconsRoot.existsSync()) {
    iconsRoot.deleteSync(recursive: true);
  }

  for (final size in _sizes) {
    final outDir = Directory('resources/icons/hicolor/${size}x$size/apps');
    outDir.createSync(recursive: true);
    final resized = img.copyResize(
      decoded,
      width: size,
      height: size,
      interpolation: img.Interpolation.cubic,
    );
    File('${outDir.path}/$_appId.png')
        .writeAsBytesSync(img.encodePng(resized));
  }

  stdout.writeln('linux hicolor icons generated under resources/icons/');
}
