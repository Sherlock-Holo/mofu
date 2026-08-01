import 'package:flutter/material.dart';
import 'package:mofu/app/app.dart';
import 'package:mofu/src/rust/api/setup.dart';
import 'package:mofu/src/rust/frb_generated.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  setup();
  runApp(const MofuApp());
}
