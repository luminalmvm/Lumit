// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'footage.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$BridgeProxyState {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeProxyState);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeProxyState()';
  }
}

/// @nodoc
class $BridgeProxyStateCopyWith<$Res> {
  $BridgeProxyStateCopyWith(
      BridgeProxyState _, $Res Function(BridgeProxyState) __);
}

/// Adds pattern-matching-related methods to [BridgeProxyState].
extension BridgeProxyStatePatterns on BridgeProxyState {
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
    TResult Function(BridgeProxyState_Idle value)? idle,
    TResult Function(BridgeProxyState_Running value)? running,
    TResult Function(BridgeProxyState_Done value)? done,
    TResult Function(BridgeProxyState_Failed value)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeProxyState_Idle() when idle != null:
        return idle(_that);
      case BridgeProxyState_Running() when running != null:
        return running(_that);
      case BridgeProxyState_Done() when done != null:
        return done(_that);
      case BridgeProxyState_Failed() when failed != null:
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
    required TResult Function(BridgeProxyState_Idle value) idle,
    required TResult Function(BridgeProxyState_Running value) running,
    required TResult Function(BridgeProxyState_Done value) done,
    required TResult Function(BridgeProxyState_Failed value) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeProxyState_Idle():
        return idle(_that);
      case BridgeProxyState_Running():
        return running(_that);
      case BridgeProxyState_Done():
        return done(_that);
      case BridgeProxyState_Failed():
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
    TResult? Function(BridgeProxyState_Idle value)? idle,
    TResult? Function(BridgeProxyState_Running value)? running,
    TResult? Function(BridgeProxyState_Done value)? done,
    TResult? Function(BridgeProxyState_Failed value)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeProxyState_Idle() when idle != null:
        return idle(_that);
      case BridgeProxyState_Running() when running != null:
        return running(_that);
      case BridgeProxyState_Done() when done != null:
        return done(_that);
      case BridgeProxyState_Failed() when failed != null:
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
    TResult Function()? idle,
    TResult Function(BigInt frame, BigInt total)? running,
    TResult Function(String path)? done,
    TResult Function(String error)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeProxyState_Idle() when idle != null:
        return idle();
      case BridgeProxyState_Running() when running != null:
        return running(_that.frame, _that.total);
      case BridgeProxyState_Done() when done != null:
        return done(_that.path);
      case BridgeProxyState_Failed() when failed != null:
        return failed(_that.error);
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
    required TResult Function() idle,
    required TResult Function(BigInt frame, BigInt total) running,
    required TResult Function(String path) done,
    required TResult Function(String error) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeProxyState_Idle():
        return idle();
      case BridgeProxyState_Running():
        return running(_that.frame, _that.total);
      case BridgeProxyState_Done():
        return done(_that.path);
      case BridgeProxyState_Failed():
        return failed(_that.error);
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
    TResult? Function()? idle,
    TResult? Function(BigInt frame, BigInt total)? running,
    TResult? Function(String path)? done,
    TResult? Function(String error)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeProxyState_Idle() when idle != null:
        return idle();
      case BridgeProxyState_Running() when running != null:
        return running(_that.frame, _that.total);
      case BridgeProxyState_Done() when done != null:
        return done(_that.path);
      case BridgeProxyState_Failed() when failed != null:
        return failed(_that.error);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeProxyState_Idle extends BridgeProxyState {
  const BridgeProxyState_Idle() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeProxyState_Idle);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeProxyState.idle()';
  }
}

/// @nodoc

class BridgeProxyState_Running extends BridgeProxyState {
  const BridgeProxyState_Running({required this.frame, required this.total})
      : super._();

  final BigInt frame;

  /// Zero until the source's length has been read.
  final BigInt total;

  /// Create a copy of BridgeProxyState
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeProxyState_RunningCopyWith<BridgeProxyState_Running> get copyWith =>
      _$BridgeProxyState_RunningCopyWithImpl<BridgeProxyState_Running>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeProxyState_Running &&
            (identical(other.frame, frame) || other.frame == frame) &&
            (identical(other.total, total) || other.total == total));
  }

  @override
  int get hashCode => Object.hash(runtimeType, frame, total);

  @override
  String toString() {
    return 'BridgeProxyState.running(frame: $frame, total: $total)';
  }
}

/// @nodoc
abstract mixin class $BridgeProxyState_RunningCopyWith<$Res>
    implements $BridgeProxyStateCopyWith<$Res> {
  factory $BridgeProxyState_RunningCopyWith(BridgeProxyState_Running value,
          $Res Function(BridgeProxyState_Running) _then) =
      _$BridgeProxyState_RunningCopyWithImpl;
  @useResult
  $Res call({BigInt frame, BigInt total});
}

/// @nodoc
class _$BridgeProxyState_RunningCopyWithImpl<$Res>
    implements $BridgeProxyState_RunningCopyWith<$Res> {
  _$BridgeProxyState_RunningCopyWithImpl(this._self, this._then);

  final BridgeProxyState_Running _self;
  final $Res Function(BridgeProxyState_Running) _then;

  /// Create a copy of BridgeProxyState
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? frame = null,
    Object? total = null,
  }) {
    return _then(BridgeProxyState_Running(
      frame: null == frame
          ? _self.frame
          : frame // ignore: cast_nullable_to_non_nullable
              as BigInt,
      total: null == total
          ? _self.total
          : total // ignore: cast_nullable_to_non_nullable
              as BigInt,
    ));
  }
}

/// @nodoc

class BridgeProxyState_Done extends BridgeProxyState {
  const BridgeProxyState_Done({required this.path}) : super._();

  final String path;

  /// Create a copy of BridgeProxyState
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeProxyState_DoneCopyWith<BridgeProxyState_Done> get copyWith =>
      _$BridgeProxyState_DoneCopyWithImpl<BridgeProxyState_Done>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeProxyState_Done &&
            (identical(other.path, path) || other.path == path));
  }

  @override
  int get hashCode => Object.hash(runtimeType, path);

  @override
  String toString() {
    return 'BridgeProxyState.done(path: $path)';
  }
}

/// @nodoc
abstract mixin class $BridgeProxyState_DoneCopyWith<$Res>
    implements $BridgeProxyStateCopyWith<$Res> {
  factory $BridgeProxyState_DoneCopyWith(BridgeProxyState_Done value,
          $Res Function(BridgeProxyState_Done) _then) =
      _$BridgeProxyState_DoneCopyWithImpl;
  @useResult
  $Res call({String path});
}

/// @nodoc
class _$BridgeProxyState_DoneCopyWithImpl<$Res>
    implements $BridgeProxyState_DoneCopyWith<$Res> {
  _$BridgeProxyState_DoneCopyWithImpl(this._self, this._then);

  final BridgeProxyState_Done _self;
  final $Res Function(BridgeProxyState_Done) _then;

  /// Create a copy of BridgeProxyState
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? path = null,
  }) {
    return _then(BridgeProxyState_Done(
      path: null == path
          ? _self.path
          : path // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeProxyState_Failed extends BridgeProxyState {
  const BridgeProxyState_Failed({required this.error}) : super._();

  final String error;

  /// Create a copy of BridgeProxyState
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeProxyState_FailedCopyWith<BridgeProxyState_Failed> get copyWith =>
      _$BridgeProxyState_FailedCopyWithImpl<BridgeProxyState_Failed>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeProxyState_Failed &&
            (identical(other.error, error) || other.error == error));
  }

  @override
  int get hashCode => Object.hash(runtimeType, error);

  @override
  String toString() {
    return 'BridgeProxyState.failed(error: $error)';
  }
}

/// @nodoc
abstract mixin class $BridgeProxyState_FailedCopyWith<$Res>
    implements $BridgeProxyStateCopyWith<$Res> {
  factory $BridgeProxyState_FailedCopyWith(BridgeProxyState_Failed value,
          $Res Function(BridgeProxyState_Failed) _then) =
      _$BridgeProxyState_FailedCopyWithImpl;
  @useResult
  $Res call({String error});
}

/// @nodoc
class _$BridgeProxyState_FailedCopyWithImpl<$Res>
    implements $BridgeProxyState_FailedCopyWith<$Res> {
  _$BridgeProxyState_FailedCopyWithImpl(this._self, this._then);

  final BridgeProxyState_Failed _self;
  final $Res Function(BridgeProxyState_Failed) _then;

  /// Create a copy of BridgeProxyState
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? error = null,
  }) {
    return _then(BridgeProxyState_Failed(
      error: null == error
          ? _self.error
          : error // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

// dart format on
