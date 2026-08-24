// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'export.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$BridgeExportQueueState {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeExportQueueState);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeExportQueueState()';
  }
}

/// @nodoc
class $BridgeExportQueueStateCopyWith<$Res> {
  $BridgeExportQueueStateCopyWith(
      BridgeExportQueueState _, $Res Function(BridgeExportQueueState) __);
}

/// Adds pattern-matching-related methods to [BridgeExportQueueState].
extension BridgeExportQueueStatePatterns on BridgeExportQueueState {
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
    TResult Function(BridgeExportQueueState_Waiting value)? waiting,
    TResult Function(BridgeExportQueueState_Running value)? running,
    TResult Function(BridgeExportQueueState_Done value)? done,
    TResult Function(BridgeExportQueueState_Failed value)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExportQueueState_Waiting() when waiting != null:
        return waiting(_that);
      case BridgeExportQueueState_Running() when running != null:
        return running(_that);
      case BridgeExportQueueState_Done() when done != null:
        return done(_that);
      case BridgeExportQueueState_Failed() when failed != null:
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
    required TResult Function(BridgeExportQueueState_Waiting value) waiting,
    required TResult Function(BridgeExportQueueState_Running value) running,
    required TResult Function(BridgeExportQueueState_Done value) done,
    required TResult Function(BridgeExportQueueState_Failed value) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExportQueueState_Waiting():
        return waiting(_that);
      case BridgeExportQueueState_Running():
        return running(_that);
      case BridgeExportQueueState_Done():
        return done(_that);
      case BridgeExportQueueState_Failed():
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
    TResult? Function(BridgeExportQueueState_Waiting value)? waiting,
    TResult? Function(BridgeExportQueueState_Running value)? running,
    TResult? Function(BridgeExportQueueState_Done value)? done,
    TResult? Function(BridgeExportQueueState_Failed value)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExportQueueState_Waiting() when waiting != null:
        return waiting(_that);
      case BridgeExportQueueState_Running() when running != null:
        return running(_that);
      case BridgeExportQueueState_Done() when done != null:
        return done(_that);
      case BridgeExportQueueState_Failed() when failed != null:
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
    TResult Function()? waiting,
    TResult Function(BigInt frame, BigInt total, String encoder)? running,
    TResult Function()? done,
    TResult Function(String error)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExportQueueState_Waiting() when waiting != null:
        return waiting();
      case BridgeExportQueueState_Running() when running != null:
        return running(_that.frame, _that.total, _that.encoder);
      case BridgeExportQueueState_Done() when done != null:
        return done();
      case BridgeExportQueueState_Failed() when failed != null:
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
    required TResult Function() waiting,
    required TResult Function(BigInt frame, BigInt total, String encoder)
        running,
    required TResult Function() done,
    required TResult Function(String error) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExportQueueState_Waiting():
        return waiting();
      case BridgeExportQueueState_Running():
        return running(_that.frame, _that.total, _that.encoder);
      case BridgeExportQueueState_Done():
        return done();
      case BridgeExportQueueState_Failed():
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
    TResult? Function()? waiting,
    TResult? Function(BigInt frame, BigInt total, String encoder)? running,
    TResult? Function()? done,
    TResult? Function(String error)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExportQueueState_Waiting() when waiting != null:
        return waiting();
      case BridgeExportQueueState_Running() when running != null:
        return running(_that.frame, _that.total, _that.encoder);
      case BridgeExportQueueState_Done() when done != null:
        return done();
      case BridgeExportQueueState_Failed() when failed != null:
        return failed(_that.error);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeExportQueueState_Waiting extends BridgeExportQueueState {
  const BridgeExportQueueState_Waiting() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeExportQueueState_Waiting);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeExportQueueState.waiting()';
  }
}

/// @nodoc

class BridgeExportQueueState_Running extends BridgeExportQueueState {
  const BridgeExportQueueState_Running(
      {required this.frame, required this.total, required this.encoder})
      : super._();

  final BigInt frame;

  /// Zero until the exporter has worked out how many there are.
  final BigInt total;

  /// The encoder actually chosen, which may not be the one asked for.
  final String encoder;

  /// Create a copy of BridgeExportQueueState
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeExportQueueState_RunningCopyWith<BridgeExportQueueState_Running>
      get copyWith => _$BridgeExportQueueState_RunningCopyWithImpl<
          BridgeExportQueueState_Running>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeExportQueueState_Running &&
            (identical(other.frame, frame) || other.frame == frame) &&
            (identical(other.total, total) || other.total == total) &&
            (identical(other.encoder, encoder) || other.encoder == encoder));
  }

  @override
  int get hashCode => Object.hash(runtimeType, frame, total, encoder);

  @override
  String toString() {
    return 'BridgeExportQueueState.running(frame: $frame, total: $total, encoder: $encoder)';
  }
}

/// @nodoc
abstract mixin class $BridgeExportQueueState_RunningCopyWith<$Res>
    implements $BridgeExportQueueStateCopyWith<$Res> {
  factory $BridgeExportQueueState_RunningCopyWith(
          BridgeExportQueueState_Running value,
          $Res Function(BridgeExportQueueState_Running) _then) =
      _$BridgeExportQueueState_RunningCopyWithImpl;
  @useResult
  $Res call({BigInt frame, BigInt total, String encoder});
}

/// @nodoc
class _$BridgeExportQueueState_RunningCopyWithImpl<$Res>
    implements $BridgeExportQueueState_RunningCopyWith<$Res> {
  _$BridgeExportQueueState_RunningCopyWithImpl(this._self, this._then);

  final BridgeExportQueueState_Running _self;
  final $Res Function(BridgeExportQueueState_Running) _then;

  /// Create a copy of BridgeExportQueueState
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? frame = null,
    Object? total = null,
    Object? encoder = null,
  }) {
    return _then(BridgeExportQueueState_Running(
      frame: null == frame
          ? _self.frame
          : frame // ignore: cast_nullable_to_non_nullable
              as BigInt,
      total: null == total
          ? _self.total
          : total // ignore: cast_nullable_to_non_nullable
              as BigInt,
      encoder: null == encoder
          ? _self.encoder
          : encoder // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeExportQueueState_Done extends BridgeExportQueueState {
  const BridgeExportQueueState_Done() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeExportQueueState_Done);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeExportQueueState.done()';
  }
}

/// @nodoc

class BridgeExportQueueState_Failed extends BridgeExportQueueState {
  const BridgeExportQueueState_Failed({required this.error}) : super._();

  final String error;

  /// Create a copy of BridgeExportQueueState
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeExportQueueState_FailedCopyWith<BridgeExportQueueState_Failed>
      get copyWith => _$BridgeExportQueueState_FailedCopyWithImpl<
          BridgeExportQueueState_Failed>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeExportQueueState_Failed &&
            (identical(other.error, error) || other.error == error));
  }

  @override
  int get hashCode => Object.hash(runtimeType, error);

  @override
  String toString() {
    return 'BridgeExportQueueState.failed(error: $error)';
  }
}

/// @nodoc
abstract mixin class $BridgeExportQueueState_FailedCopyWith<$Res>
    implements $BridgeExportQueueStateCopyWith<$Res> {
  factory $BridgeExportQueueState_FailedCopyWith(
          BridgeExportQueueState_Failed value,
          $Res Function(BridgeExportQueueState_Failed) _then) =
      _$BridgeExportQueueState_FailedCopyWithImpl;
  @useResult
  $Res call({String error});
}

/// @nodoc
class _$BridgeExportQueueState_FailedCopyWithImpl<$Res>
    implements $BridgeExportQueueState_FailedCopyWith<$Res> {
  _$BridgeExportQueueState_FailedCopyWithImpl(this._self, this._then);

  final BridgeExportQueueState_Failed _self;
  final $Res Function(BridgeExportQueueState_Failed) _then;

  /// Create a copy of BridgeExportQueueState
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? error = null,
  }) {
    return _then(BridgeExportQueueState_Failed(
      error: null == error
          ? _self.error
          : error // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc
mixin _$BridgeExportState {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeExportState);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeExportState()';
  }
}

/// @nodoc
class $BridgeExportStateCopyWith<$Res> {
  $BridgeExportStateCopyWith(
      BridgeExportState _, $Res Function(BridgeExportState) __);
}

/// Adds pattern-matching-related methods to [BridgeExportState].
extension BridgeExportStatePatterns on BridgeExportState {
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
    TResult Function(BridgeExportState_Idle value)? idle,
    TResult Function(BridgeExportState_Running value)? running,
    TResult Function(BridgeExportState_Done value)? done,
    TResult Function(BridgeExportState_Failed value)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExportState_Idle() when idle != null:
        return idle(_that);
      case BridgeExportState_Running() when running != null:
        return running(_that);
      case BridgeExportState_Done() when done != null:
        return done(_that);
      case BridgeExportState_Failed() when failed != null:
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
    required TResult Function(BridgeExportState_Idle value) idle,
    required TResult Function(BridgeExportState_Running value) running,
    required TResult Function(BridgeExportState_Done value) done,
    required TResult Function(BridgeExportState_Failed value) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExportState_Idle():
        return idle(_that);
      case BridgeExportState_Running():
        return running(_that);
      case BridgeExportState_Done():
        return done(_that);
      case BridgeExportState_Failed():
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
    TResult? Function(BridgeExportState_Idle value)? idle,
    TResult? Function(BridgeExportState_Running value)? running,
    TResult? Function(BridgeExportState_Done value)? done,
    TResult? Function(BridgeExportState_Failed value)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExportState_Idle() when idle != null:
        return idle(_that);
      case BridgeExportState_Running() when running != null:
        return running(_that);
      case BridgeExportState_Done() when done != null:
        return done(_that);
      case BridgeExportState_Failed() when failed != null:
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
    TResult Function(BigInt frame, BigInt total, String encoder)? running,
    TResult Function(String path)? done,
    TResult Function(String error)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExportState_Idle() when idle != null:
        return idle();
      case BridgeExportState_Running() when running != null:
        return running(_that.frame, _that.total, _that.encoder);
      case BridgeExportState_Done() when done != null:
        return done(_that.path);
      case BridgeExportState_Failed() when failed != null:
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
    required TResult Function(BigInt frame, BigInt total, String encoder)
        running,
    required TResult Function(String path) done,
    required TResult Function(String error) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExportState_Idle():
        return idle();
      case BridgeExportState_Running():
        return running(_that.frame, _that.total, _that.encoder);
      case BridgeExportState_Done():
        return done(_that.path);
      case BridgeExportState_Failed():
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
    TResult? Function(BigInt frame, BigInt total, String encoder)? running,
    TResult? Function(String path)? done,
    TResult? Function(String error)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeExportState_Idle() when idle != null:
        return idle();
      case BridgeExportState_Running() when running != null:
        return running(_that.frame, _that.total, _that.encoder);
      case BridgeExportState_Done() when done != null:
        return done(_that.path);
      case BridgeExportState_Failed() when failed != null:
        return failed(_that.error);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeExportState_Idle extends BridgeExportState {
  const BridgeExportState_Idle() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeExportState_Idle);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeExportState.idle()';
  }
}

/// @nodoc

class BridgeExportState_Running extends BridgeExportState {
  const BridgeExportState_Running(
      {required this.frame, required this.total, required this.encoder})
      : super._();

  final BigInt frame;

  /// Zero until the exporter has worked out how many there are.
  final BigInt total;

  /// The encoder actually chosen, which may not be the one asked for —
  /// a hardware encoder that is not there falls back to software, and the
  /// dialogue should say so rather than claim what was requested.
  final String encoder;

  /// Create a copy of BridgeExportState
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeExportState_RunningCopyWith<BridgeExportState_Running> get copyWith =>
      _$BridgeExportState_RunningCopyWithImpl<BridgeExportState_Running>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeExportState_Running &&
            (identical(other.frame, frame) || other.frame == frame) &&
            (identical(other.total, total) || other.total == total) &&
            (identical(other.encoder, encoder) || other.encoder == encoder));
  }

  @override
  int get hashCode => Object.hash(runtimeType, frame, total, encoder);

  @override
  String toString() {
    return 'BridgeExportState.running(frame: $frame, total: $total, encoder: $encoder)';
  }
}

/// @nodoc
abstract mixin class $BridgeExportState_RunningCopyWith<$Res>
    implements $BridgeExportStateCopyWith<$Res> {
  factory $BridgeExportState_RunningCopyWith(BridgeExportState_Running value,
          $Res Function(BridgeExportState_Running) _then) =
      _$BridgeExportState_RunningCopyWithImpl;
  @useResult
  $Res call({BigInt frame, BigInt total, String encoder});
}

/// @nodoc
class _$BridgeExportState_RunningCopyWithImpl<$Res>
    implements $BridgeExportState_RunningCopyWith<$Res> {
  _$BridgeExportState_RunningCopyWithImpl(this._self, this._then);

  final BridgeExportState_Running _self;
  final $Res Function(BridgeExportState_Running) _then;

  /// Create a copy of BridgeExportState
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? frame = null,
    Object? total = null,
    Object? encoder = null,
  }) {
    return _then(BridgeExportState_Running(
      frame: null == frame
          ? _self.frame
          : frame // ignore: cast_nullable_to_non_nullable
              as BigInt,
      total: null == total
          ? _self.total
          : total // ignore: cast_nullable_to_non_nullable
              as BigInt,
      encoder: null == encoder
          ? _self.encoder
          : encoder // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeExportState_Done extends BridgeExportState {
  const BridgeExportState_Done({required this.path}) : super._();

  final String path;

  /// Create a copy of BridgeExportState
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeExportState_DoneCopyWith<BridgeExportState_Done> get copyWith =>
      _$BridgeExportState_DoneCopyWithImpl<BridgeExportState_Done>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeExportState_Done &&
            (identical(other.path, path) || other.path == path));
  }

  @override
  int get hashCode => Object.hash(runtimeType, path);

  @override
  String toString() {
    return 'BridgeExportState.done(path: $path)';
  }
}

/// @nodoc
abstract mixin class $BridgeExportState_DoneCopyWith<$Res>
    implements $BridgeExportStateCopyWith<$Res> {
  factory $BridgeExportState_DoneCopyWith(BridgeExportState_Done value,
          $Res Function(BridgeExportState_Done) _then) =
      _$BridgeExportState_DoneCopyWithImpl;
  @useResult
  $Res call({String path});
}

/// @nodoc
class _$BridgeExportState_DoneCopyWithImpl<$Res>
    implements $BridgeExportState_DoneCopyWith<$Res> {
  _$BridgeExportState_DoneCopyWithImpl(this._self, this._then);

  final BridgeExportState_Done _self;
  final $Res Function(BridgeExportState_Done) _then;

  /// Create a copy of BridgeExportState
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? path = null,
  }) {
    return _then(BridgeExportState_Done(
      path: null == path
          ? _self.path
          : path // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeExportState_Failed extends BridgeExportState {
  const BridgeExportState_Failed({required this.error}) : super._();

  final String error;

  /// Create a copy of BridgeExportState
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeExportState_FailedCopyWith<BridgeExportState_Failed> get copyWith =>
      _$BridgeExportState_FailedCopyWithImpl<BridgeExportState_Failed>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeExportState_Failed &&
            (identical(other.error, error) || other.error == error));
  }

  @override
  int get hashCode => Object.hash(runtimeType, error);

  @override
  String toString() {
    return 'BridgeExportState.failed(error: $error)';
  }
}

/// @nodoc
abstract mixin class $BridgeExportState_FailedCopyWith<$Res>
    implements $BridgeExportStateCopyWith<$Res> {
  factory $BridgeExportState_FailedCopyWith(BridgeExportState_Failed value,
          $Res Function(BridgeExportState_Failed) _then) =
      _$BridgeExportState_FailedCopyWithImpl;
  @useResult
  $Res call({String error});
}

/// @nodoc
class _$BridgeExportState_FailedCopyWithImpl<$Res>
    implements $BridgeExportState_FailedCopyWith<$Res> {
  _$BridgeExportState_FailedCopyWithImpl(this._self, this._then);

  final BridgeExportState_Failed _self;
  final $Res Function(BridgeExportState_Failed) _then;

  /// Create a copy of BridgeExportState
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? error = null,
  }) {
    return _then(BridgeExportState_Failed(
      error: null == error
          ? _self.error
          : error // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

// dart format on
