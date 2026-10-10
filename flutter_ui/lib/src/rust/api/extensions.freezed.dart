// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'extensions.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$BridgeExtensionOutcome {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeExtensionOutcome);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeExtensionOutcome()';
  }
}

/// @nodoc
class $BridgeExtensionOutcomeCopyWith<$Res> {
  $BridgeExtensionOutcomeCopyWith(
      BridgeExtensionOutcome _, $Res Function(BridgeExtensionOutcome) __);
}

/// Adds pattern-matching-related methods to [BridgeExtensionOutcome].
extension BridgeExtensionOutcomePatterns on BridgeExtensionOutcome {
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
    TResult Function(BridgeExtensionOutcome_Ready value)? ready,
    TResult Function(BridgeExtensionOutcome_NotAnExtension value)?
        notAnExtension,
    TResult Function(BridgeExtensionOutcome_Invalid value)? invalid,
    TResult Function(BridgeExtensionOutcome_Newer value)? newer,
    TResult Function(BridgeExtensionOutcome_TooLarge value)? tooLarge,
    TResult Function(BridgeExtensionOutcome_Busy value)? busy,
    TResult Function(BridgeExtensionOutcome_Failed value)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExtensionOutcome_Ready() when ready != null:
        return ready(_that);
      case BridgeExtensionOutcome_NotAnExtension() when notAnExtension != null:
        return notAnExtension(_that);
      case BridgeExtensionOutcome_Invalid() when invalid != null:
        return invalid(_that);
      case BridgeExtensionOutcome_Newer() when newer != null:
        return newer(_that);
      case BridgeExtensionOutcome_TooLarge() when tooLarge != null:
        return tooLarge(_that);
      case BridgeExtensionOutcome_Busy() when busy != null:
        return busy(_that);
      case BridgeExtensionOutcome_Failed() when failed != null:
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
    required TResult Function(BridgeExtensionOutcome_Ready value) ready,
    required TResult Function(BridgeExtensionOutcome_NotAnExtension value)
        notAnExtension,
    required TResult Function(BridgeExtensionOutcome_Invalid value) invalid,
    required TResult Function(BridgeExtensionOutcome_Newer value) newer,
    required TResult Function(BridgeExtensionOutcome_TooLarge value) tooLarge,
    required TResult Function(BridgeExtensionOutcome_Busy value) busy,
    required TResult Function(BridgeExtensionOutcome_Failed value) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExtensionOutcome_Ready():
        return ready(_that);
      case BridgeExtensionOutcome_NotAnExtension():
        return notAnExtension(_that);
      case BridgeExtensionOutcome_Invalid():
        return invalid(_that);
      case BridgeExtensionOutcome_Newer():
        return newer(_that);
      case BridgeExtensionOutcome_TooLarge():
        return tooLarge(_that);
      case BridgeExtensionOutcome_Busy():
        return busy(_that);
      case BridgeExtensionOutcome_Failed():
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
    TResult? Function(BridgeExtensionOutcome_Ready value)? ready,
    TResult? Function(BridgeExtensionOutcome_NotAnExtension value)?
        notAnExtension,
    TResult? Function(BridgeExtensionOutcome_Invalid value)? invalid,
    TResult? Function(BridgeExtensionOutcome_Newer value)? newer,
    TResult? Function(BridgeExtensionOutcome_TooLarge value)? tooLarge,
    TResult? Function(BridgeExtensionOutcome_Busy value)? busy,
    TResult? Function(BridgeExtensionOutcome_Failed value)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExtensionOutcome_Ready() when ready != null:
        return ready(_that);
      case BridgeExtensionOutcome_NotAnExtension() when notAnExtension != null:
        return notAnExtension(_that);
      case BridgeExtensionOutcome_Invalid() when invalid != null:
        return invalid(_that);
      case BridgeExtensionOutcome_Newer() when newer != null:
        return newer(_that);
      case BridgeExtensionOutcome_TooLarge() when tooLarge != null:
        return tooLarge(_that);
      case BridgeExtensionOutcome_Busy() when busy != null:
        return busy(_that);
      case BridgeExtensionOutcome_Failed() when failed != null:
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
    TResult Function(BridgeExtension extension_)? ready,
    TResult Function()? notAnExtension,
    TResult Function(String why)? invalid,
    TResult Function(String version)? newer,
    TResult Function()? tooLarge,
    TResult Function()? busy,
    TResult Function()? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExtensionOutcome_Ready() when ready != null:
        return ready(_that.extension_);
      case BridgeExtensionOutcome_NotAnExtension() when notAnExtension != null:
        return notAnExtension();
      case BridgeExtensionOutcome_Invalid() when invalid != null:
        return invalid(_that.why);
      case BridgeExtensionOutcome_Newer() when newer != null:
        return newer(_that.version);
      case BridgeExtensionOutcome_TooLarge() when tooLarge != null:
        return tooLarge();
      case BridgeExtensionOutcome_Busy() when busy != null:
        return busy();
      case BridgeExtensionOutcome_Failed() when failed != null:
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
    required TResult Function(BridgeExtension extension_) ready,
    required TResult Function() notAnExtension,
    required TResult Function(String why) invalid,
    required TResult Function(String version) newer,
    required TResult Function() tooLarge,
    required TResult Function() busy,
    required TResult Function() failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExtensionOutcome_Ready():
        return ready(_that.extension_);
      case BridgeExtensionOutcome_NotAnExtension():
        return notAnExtension();
      case BridgeExtensionOutcome_Invalid():
        return invalid(_that.why);
      case BridgeExtensionOutcome_Newer():
        return newer(_that.version);
      case BridgeExtensionOutcome_TooLarge():
        return tooLarge();
      case BridgeExtensionOutcome_Busy():
        return busy();
      case BridgeExtensionOutcome_Failed():
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
    TResult? Function(BridgeExtension extension_)? ready,
    TResult? Function()? notAnExtension,
    TResult? Function(String why)? invalid,
    TResult? Function(String version)? newer,
    TResult? Function()? tooLarge,
    TResult? Function()? busy,
    TResult? Function()? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExtensionOutcome_Ready() when ready != null:
        return ready(_that.extension_);
      case BridgeExtensionOutcome_NotAnExtension() when notAnExtension != null:
        return notAnExtension();
      case BridgeExtensionOutcome_Invalid() when invalid != null:
        return invalid(_that.why);
      case BridgeExtensionOutcome_Newer() when newer != null:
        return newer(_that.version);
      case BridgeExtensionOutcome_TooLarge() when tooLarge != null:
        return tooLarge();
      case BridgeExtensionOutcome_Busy() when busy != null:
        return busy();
      case BridgeExtensionOutcome_Failed() when failed != null:
        return failed();
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeExtensionOutcome_Ready extends BridgeExtensionOutcome {
  const BridgeExtensionOutcome_Ready({required this.extension_}) : super._();

  final BridgeExtension extension_;

  /// Create a copy of BridgeExtensionOutcome
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeExtensionOutcome_ReadyCopyWith<BridgeExtensionOutcome_Ready>
      get copyWith => _$BridgeExtensionOutcome_ReadyCopyWithImpl<
          BridgeExtensionOutcome_Ready>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeExtensionOutcome_Ready &&
            (identical(other.extension_, extension_) ||
                other.extension_ == extension_));
  }

  @override
  int get hashCode => Object.hash(runtimeType, extension_);

  @override
  String toString() {
    return 'BridgeExtensionOutcome.ready(extension_: $extension_)';
  }
}

/// @nodoc
abstract mixin class $BridgeExtensionOutcome_ReadyCopyWith<$Res>
    implements $BridgeExtensionOutcomeCopyWith<$Res> {
  factory $BridgeExtensionOutcome_ReadyCopyWith(
          BridgeExtensionOutcome_Ready value,
          $Res Function(BridgeExtensionOutcome_Ready) _then) =
      _$BridgeExtensionOutcome_ReadyCopyWithImpl;
  @useResult
  $Res call({BridgeExtension extension_});
}

/// @nodoc
class _$BridgeExtensionOutcome_ReadyCopyWithImpl<$Res>
    implements $BridgeExtensionOutcome_ReadyCopyWith<$Res> {
  _$BridgeExtensionOutcome_ReadyCopyWithImpl(this._self, this._then);

  final BridgeExtensionOutcome_Ready _self;
  final $Res Function(BridgeExtensionOutcome_Ready) _then;

  /// Create a copy of BridgeExtensionOutcome
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? extension_ = null,
  }) {
    return _then(BridgeExtensionOutcome_Ready(
      extension_: null == extension_
          ? _self.extension_
          : extension_ // ignore: cast_nullable_to_non_nullable
              as BridgeExtension,
    ));
  }
}

/// @nodoc

class BridgeExtensionOutcome_NotAnExtension extends BridgeExtensionOutcome {
  const BridgeExtensionOutcome_NotAnExtension() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeExtensionOutcome_NotAnExtension);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeExtensionOutcome.notAnExtension()';
  }
}

/// @nodoc

class BridgeExtensionOutcome_Invalid extends BridgeExtensionOutcome {
  const BridgeExtensionOutcome_Invalid({required this.why}) : super._();

  final String why;

  /// Create a copy of BridgeExtensionOutcome
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeExtensionOutcome_InvalidCopyWith<BridgeExtensionOutcome_Invalid>
      get copyWith => _$BridgeExtensionOutcome_InvalidCopyWithImpl<
          BridgeExtensionOutcome_Invalid>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeExtensionOutcome_Invalid &&
            (identical(other.why, why) || other.why == why));
  }

  @override
  int get hashCode => Object.hash(runtimeType, why);

  @override
  String toString() {
    return 'BridgeExtensionOutcome.invalid(why: $why)';
  }
}

/// @nodoc
abstract mixin class $BridgeExtensionOutcome_InvalidCopyWith<$Res>
    implements $BridgeExtensionOutcomeCopyWith<$Res> {
  factory $BridgeExtensionOutcome_InvalidCopyWith(
          BridgeExtensionOutcome_Invalid value,
          $Res Function(BridgeExtensionOutcome_Invalid) _then) =
      _$BridgeExtensionOutcome_InvalidCopyWithImpl;
  @useResult
  $Res call({String why});
}

/// @nodoc
class _$BridgeExtensionOutcome_InvalidCopyWithImpl<$Res>
    implements $BridgeExtensionOutcome_InvalidCopyWith<$Res> {
  _$BridgeExtensionOutcome_InvalidCopyWithImpl(this._self, this._then);

  final BridgeExtensionOutcome_Invalid _self;
  final $Res Function(BridgeExtensionOutcome_Invalid) _then;

  /// Create a copy of BridgeExtensionOutcome
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? why = null,
  }) {
    return _then(BridgeExtensionOutcome_Invalid(
      why: null == why
          ? _self.why
          : why // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeExtensionOutcome_Newer extends BridgeExtensionOutcome {
  const BridgeExtensionOutcome_Newer({required this.version}) : super._();

  final String version;

  /// Create a copy of BridgeExtensionOutcome
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeExtensionOutcome_NewerCopyWith<BridgeExtensionOutcome_Newer>
      get copyWith => _$BridgeExtensionOutcome_NewerCopyWithImpl<
          BridgeExtensionOutcome_Newer>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeExtensionOutcome_Newer &&
            (identical(other.version, version) || other.version == version));
  }

  @override
  int get hashCode => Object.hash(runtimeType, version);

  @override
  String toString() {
    return 'BridgeExtensionOutcome.newer(version: $version)';
  }
}

/// @nodoc
abstract mixin class $BridgeExtensionOutcome_NewerCopyWith<$Res>
    implements $BridgeExtensionOutcomeCopyWith<$Res> {
  factory $BridgeExtensionOutcome_NewerCopyWith(
          BridgeExtensionOutcome_Newer value,
          $Res Function(BridgeExtensionOutcome_Newer) _then) =
      _$BridgeExtensionOutcome_NewerCopyWithImpl;
  @useResult
  $Res call({String version});
}

/// @nodoc
class _$BridgeExtensionOutcome_NewerCopyWithImpl<$Res>
    implements $BridgeExtensionOutcome_NewerCopyWith<$Res> {
  _$BridgeExtensionOutcome_NewerCopyWithImpl(this._self, this._then);

  final BridgeExtensionOutcome_Newer _self;
  final $Res Function(BridgeExtensionOutcome_Newer) _then;

  /// Create a copy of BridgeExtensionOutcome
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? version = null,
  }) {
    return _then(BridgeExtensionOutcome_Newer(
      version: null == version
          ? _self.version
          : version // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeExtensionOutcome_TooLarge extends BridgeExtensionOutcome {
  const BridgeExtensionOutcome_TooLarge() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeExtensionOutcome_TooLarge);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeExtensionOutcome.tooLarge()';
  }
}

/// @nodoc

class BridgeExtensionOutcome_Busy extends BridgeExtensionOutcome {
  const BridgeExtensionOutcome_Busy() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeExtensionOutcome_Busy);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeExtensionOutcome.busy()';
  }
}

/// @nodoc

class BridgeExtensionOutcome_Failed extends BridgeExtensionOutcome {
  const BridgeExtensionOutcome_Failed() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeExtensionOutcome_Failed);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeExtensionOutcome.failed()';
  }
}

// dart format on
