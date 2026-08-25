import 'dart:convert';

import 'package:lumit_flutter/src/rust/api/expressions.dart';

class ExpressionsMetadata {
  // Empty until [load] has run, rather than uninitialised: anything that offers
  // completions is built before the engine has answered, and an empty list is a
  // field with no suggestions yet, not a crash.
  static ExpressionsApi api = ExpressionsApi(functions: []);

  static Future<void> load() async {
    var data = await Expressions.getExpressionsMetadata();
    final parsed = jsonDecode(data);
    var a = ExpressionsApi.fromJson(parsed);
    final regExp = RegExp(
        r'[\^$*.\[\]{}()?\-"!@#%&/\,><:;_~`+=' // <-- Notice the escaped symbols
        "'" // <-- ' is added to the expression
        ']');


    for(var f in a.functions) {
      if(f.name.startsWith("get\$")) {

        final type = f.params[0].type.replaceAll("&mut ", "");

        f.name = f.name.replaceAll("get\$", "$type.");
        f.signature = f.signature.replaceAll("get\$", "");
        f.isGetter = true;
      }
    }

    a.functions.removeWhere((i) => i.name.startsWith(regExp));

    api = a;
  }
}

class ExpressionsApi {
  final List<FunctionDef> functions;

  ExpressionsApi({
    required this.functions,
  });

  factory ExpressionsApi.fromJson(Map<String, dynamic> json) {
    return ExpressionsApi(
      functions: (json['functions'] as List<dynamic>)
          .map((i) => FunctionDef.fromJson(i))
          .toList(),
    );
  }
}

class FunctionDef {
  int baseHash;
  int fullHash;
  String namespace;
  String access;
  String name;
  bool isAnonymous;
  String type;
  int numParams;
  List<Parameter> params;
  String? returnType;
  List<String> docComments;
  String signature;
  bool isGetter;

  FunctionDef({
    required this.baseHash,
    required this.fullHash,
    required this.namespace,
    required this.access,
    required this.name,
    required this.isAnonymous,
    required this.type,
    required this.numParams,
    required this.params,
    required this.docComments,
    required this.returnType,
    required this.signature,
    this.isGetter = false,
  });

  factory FunctionDef.fromJson(Map<String, dynamic> json) {
    return FunctionDef(
      baseHash: json['baseHash'].toInt(),
      fullHash: json['fullHash'].toInt(),
      namespace: json['namespace'],
      access: json['access'],
      name: json['name'],
      isAnonymous: json['isAnonymous'],
      type: json['type'],
      numParams: json['numParams'].toInt(),
      params: json.containsKey("params")
          ? (json['params'] as List<dynamic>)
              .map((i) => Parameter.fromJson(i))
              .toList()
          : [],
      returnType: json['returnType'],
      docComments: (json['docComments'] as List<dynamic>? ?? [])
          .map((i) => i.toString())
          .toList(),
      signature: (json['signature'] as String)
          .replaceAll(": Dynamic", "") // dont show types for dynamic args
          .replaceAll("&mut ", ""), // expression users dont care about mutability
    );
  }
}

class Parameter {
  final String name;
  final String type;

  Parameter({
    required this.name,
    required this.type,
  });

  factory Parameter.fromJson(Map<String, dynamic> json) {
    return Parameter(
      name: json['name'] ?? "",
      type: json['type'] ?? "",
    );
  }
}
