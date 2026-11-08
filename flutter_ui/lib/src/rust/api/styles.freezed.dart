// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'styles.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$BridgeStyleOutcome {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeStyleOutcome);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeStyleOutcome()';
  }
}

/// @nodoc
class $BridgeStyleOutcomeCopyWith<$Res> {
  $BridgeStyleOutcomeCopyWith(
      BridgeStyleOutcome _, $Res Function(BridgeStyleOutcome) __);
}

/// Adds pattern-matching-related methods to [BridgeStyleOutcome].
extension BridgeStyleOutcomePatterns on BridgeStyleOutcome {
  /// A variant of `map` that fallback to returning `orElse`.
  ///
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case final Subclass value:
  ///     return ...;
  ///   case _:
  ///     return orElse();
  /// }
  /// ```

  @optionalTypeArgs
  TResult maybeMap<TResult extends Object?>({
    TResult Function(BridgeStyleOutcome_Ready value)? ready,
    TResult Function(BridgeStyleOutcome_NotAStyle value)? notAStyle,
    TResult Function(BridgeStyleOutcome_TooLarge value)? tooLarge,
    TResult Function(BridgeStyleOutcome_Unsafe value)? unsafe,
    TResult Function(BridgeStyleOutcome_Failed value)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeStyleOutcome_Ready() when ready != null:
        return ready(_that);
      case BridgeStyleOutcome_NotAStyle() when notAStyle != null:
        return notAStyle(_that);
      case BridgeStyleOutcome_TooLarge() when tooLarge != null:
        return tooLarge(_that);
      case BridgeStyleOutcome_Unsafe() when unsafe != null:
        return unsafe(_that);
      case BridgeStyleOutcome_Failed() when failed != null:
        return failed(_that);
      case _:
        return orElse();
    }
  }

  /// A `switch`-like method, using callbacks.
  ///
  /// Callbacks receives the raw object, upcasted.
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case final Subclass value:
  ///     return ...;
  ///   case final Subclass2 value:
  ///     return ...;
  /// }
  /// ```

  @optionalTypeArgs
  TResult map<TResult extends Object?>({
    required TResult Function(BridgeStyleOutcome_Ready value) ready,
    required TResult Function(BridgeStyleOutcome_NotAStyle value) notAStyle,
    required TResult Function(BridgeStyleOutcome_TooLarge value) tooLarge,
    required TResult Function(BridgeStyleOutcome_Unsafe value) unsafe,
    required TResult Function(BridgeStyleOutcome_Failed value) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeStyleOutcome_Ready():
        return ready(_that);
      case BridgeStyleOutcome_NotAStyle():
        return notAStyle(_that);
      case BridgeStyleOutcome_TooLarge():
        return tooLarge(_that);
      case BridgeStyleOutcome_Unsafe():
        return unsafe(_that);
      case BridgeStyleOutcome_Failed():
        return failed(_that);
    }
  }

  /// A variant of `map` that fallback to returning `null`.
  ///
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case final Subclass value:
  ///     return ...;
  ///   case _:
  ///     return null;
  /// }
  /// ```

  @optionalTypeArgs
  TResult? mapOrNull<TResult extends Object?>({
    TResult? Function(BridgeStyleOutcome_Ready value)? ready,
    TResult? Function(BridgeStyleOutcome_NotAStyle value)? notAStyle,
    TResult? Function(BridgeStyleOutcome_TooLarge value)? tooLarge,
    TResult? Function(BridgeStyleOutcome_Unsafe value)? unsafe,
    TResult? Function(BridgeStyleOutcome_Failed value)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeStyleOutcome_Ready() when ready != null:
        return ready(_that);
      case BridgeStyleOutcome_NotAStyle() when notAStyle != null:
        return notAStyle(_that);
      case BridgeStyleOutcome_TooLarge() when tooLarge != null:
        return tooLarge(_that);
      case BridgeStyleOutcome_Unsafe() when unsafe != null:
        return unsafe(_that);
      case BridgeStyleOutcome_Failed() when failed != null:
        return failed(_that);
      case _:
        return null;
    }
  }

  /// A variant of `when` that fallback to an `orElse` callback.
  ///
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case Subclass(:final field):
  ///     return ...;
  ///   case _:
  ///     return orElse();
  /// }
  /// ```

  @optionalTypeArgs
  TResult maybeWhen<TResult extends Object?>({
    TResult Function(String name)? ready,
    TResult Function()? notAStyle,
    TResult Function()? tooLarge,
    TResult Function()? unsafe,
    TResult Function()? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeStyleOutcome_Ready() when ready != null:
        return ready(_that.name);
      case BridgeStyleOutcome_NotAStyle() when notAStyle != null:
        return notAStyle();
      case BridgeStyleOutcome_TooLarge() when tooLarge != null:
        return tooLarge();
      case BridgeStyleOutcome_Unsafe() when unsafe != null:
        return unsafe();
      case BridgeStyleOutcome_Failed() when failed != null:
        return failed();
      case _:
        return orElse();
    }
  }

  /// A `switch`-like method, using callbacks.
  ///
  /// As opposed to `map`, this offers destructuring.
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case Subclass(:final field):
  ///     return ...;
  ///   case Subclass2(:final field2):
  ///     return ...;
  /// }
  /// ```

  @optionalTypeArgs
  TResult when<TResult extends Object?>({
    required TResult Function(String name) ready,
    required TResult Function() notAStyle,
    required TResult Function() tooLarge,
    required TResult Function() unsafe,
    required TResult Function() failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeStyleOutcome_Ready():
        return ready(_that.name);
      case BridgeStyleOutcome_NotAStyle():
        return notAStyle();
      case BridgeStyleOutcome_TooLarge():
        return tooLarge();
      case BridgeStyleOutcome_Unsafe():
        return unsafe();
      case BridgeStyleOutcome_Failed():
        return failed();
    }
  }

  /// A variant of `when` that fallback to returning `null`
  ///
  /// It is equivalent to doing:
  /// ```dart
  /// switch (sealedClass) {
  ///   case Subclass(:final field):
  ///     return ...;
  ///   case _:
  ///     return null;
  /// }
  /// ```

  @optionalTypeArgs
  TResult? whenOrNull<TResult extends Object?>({
    TResult? Function(String name)? ready,
    TResult? Function()? notAStyle,
    TResult? Function()? tooLarge,
    TResult? Function()? unsafe,
    TResult? Function()? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeStyleOutcome_Ready() when ready != null:
        return ready(_that.name);
      case BridgeStyleOutcome_NotAStyle() when notAStyle != null:
        return notAStyle();
      case BridgeStyleOutcome_TooLarge() when tooLarge != null:
        return tooLarge();
      case BridgeStyleOutcome_Unsafe() when unsafe != null:
        return unsafe();
      case BridgeStyleOutcome_Failed() when failed != null:
        return failed();
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeStyleOutcome_Ready extends BridgeStyleOutcome {
  const BridgeStyleOutcome_Ready({required this.name}) : super._();

  final String name;

  /// Create a copy of BridgeStyleOutcome
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeStyleOutcome_ReadyCopyWith<BridgeStyleOutcome_Ready> get copyWith =>
      _$BridgeStyleOutcome_ReadyCopyWithImpl<BridgeStyleOutcome_Ready>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeStyleOutcome_Ready &&
            (identical(other.name, name) || other.name == name));
  }

  @override
  int get hashCode => Object.hash(runtimeType, name);

  @override
  String toString() {
    return 'BridgeStyleOutcome.ready(name: $name)';
  }
}

/// @nodoc
abstract mixin class $BridgeStyleOutcome_ReadyCopyWith<$Res>
    implements $BridgeStyleOutcomeCopyWith<$Res> {
  factory $BridgeStyleOutcome_ReadyCopyWith(BridgeStyleOutcome_Ready value,
          $Res Function(BridgeStyleOutcome_Ready) _then) =
      _$BridgeStyleOutcome_ReadyCopyWithImpl;
  @useResult
  $Res call({String name});
}

/// @nodoc
class _$BridgeStyleOutcome_ReadyCopyWithImpl<$Res>
    implements $BridgeStyleOutcome_ReadyCopyWith<$Res> {
  _$BridgeStyleOutcome_ReadyCopyWithImpl(this._self, this._then);

  final BridgeStyleOutcome_Ready _self;
  final $Res Function(BridgeStyleOutcome_Ready) _then;

  /// Create a copy of BridgeStyleOutcome
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? name = null,
  }) {
    return _then(BridgeStyleOutcome_Ready(
      name: null == name
          ? _self.name
          : name // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeStyleOutcome_NotAStyle extends BridgeStyleOutcome {
  const BridgeStyleOutcome_NotAStyle() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeStyleOutcome_NotAStyle);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeStyleOutcome.notAStyle()';
  }
}

/// @nodoc

class BridgeStyleOutcome_TooLarge extends BridgeStyleOutcome {
  const BridgeStyleOutcome_TooLarge() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeStyleOutcome_TooLarge);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeStyleOutcome.tooLarge()';
  }
}

/// @nodoc

class BridgeStyleOutcome_Unsafe extends BridgeStyleOutcome {
  const BridgeStyleOutcome_Unsafe() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeStyleOutcome_Unsafe);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeStyleOutcome.unsafe()';
  }
}

/// @nodoc

class BridgeStyleOutcome_Failed extends BridgeStyleOutcome {
  const BridgeStyleOutcome_Failed() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeStyleOutcome_Failed);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeStyleOutcome.failed()';
  }
}

// dart format on
