import 'package:flutter/material.dart';
import 'package:testflying/pages/home.dart';
import 'package:testflying/services/testflight_service_factory.dart';

void main() {
  runApp(const MyApp());
}

class MyApp extends StatelessWidget {
  const MyApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'testflying',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(seedColor: const Color(0xFF2478FF)),
        fontFamily: 'SF Pro Display',
        scaffoldBackgroundColor: const Color(0xFFF5F6FA),
        useMaterial3: true,
      ),
      home: HomePage(service: buildTestFlightService()),
    );
  }
}
