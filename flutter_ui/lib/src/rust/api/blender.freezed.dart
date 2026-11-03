// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'blender.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$BridgeBlenderState {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeBlenderState);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeBlenderState()';
  }
}

/// @nodoc
class $BridgeBlenderStateCopyWith<$Res> {
  $BridgeBlenderStateCopyWith(
      BridgeBlenderState _, $Res Function(BridgeBlenderState) __);
}

/// Adds pattern-matching-related methods to [BridgeBlenderState].
extension BridgeBlenderStatePatterns on BridgeBlenderState {
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
    TResult Function(BridgeBlenderState_Converting value)? converting,
    TResult Function(BridgeBlenderState_NoBlender value)? noBlender,
    TResult Function(BridgeBlenderState_Failed value)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeBlenderState_Converting() when converting != null:
        return converting(_that);
      case BridgeBlenderState_NoBlender() when noBlender != null:
        return noBlender(_that);
      case BridgeBlenderState_Failed() when failed != null:
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
    required TResult Function(BridgeBlenderState_Converting value) converting,
    required TResult Function(BridgeBlenderState_NoBlender value) noBlender,
    required TResult Function(BridgeBlenderState_Failed value) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeBlenderState_Converting():
        return converting(_that);
      case BridgeBlenderState_NoBlender():
        return noBlender(_that);
      case BridgeBlenderState_Failed():
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
    TResult? Function(BridgeBlenderState_Converting value)? converting,
    TResult? Function(BridgeBlenderState_NoBlender value)? noBlender,
    TResult? Function(BridgeBlenderState_Failed value)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeBlenderState_Converting() when converting != null:
        return converting(_that);
      case BridgeBlenderState_NoBlender() when noBlender != null:
        return noBlender(_that);
      case BridgeBlenderState_Failed() when failed != null:
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
    TResult Function()? converting,
    TResult Function()? noBlender,
    TResult Function(String reason)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeBlenderState_Converting() when converting != null:
        return converting();
      case BridgeBlenderState_NoBlender() when noBlender != null:
        return noBlender();
      case BridgeBlenderState_Failed() when failed != null:
        return failed(_that.reason);
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
    required TResult Function() converting,
    required TResult Function() noBlender,
    required TResult Function(String reason) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeBlenderState_Converting():
        return converting();
      case BridgeBlenderState_NoBlender():
        return noBlender();
      case BridgeBlenderState_Failed():
        return failed(_that.reason);
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
    TResult? Function()? converting,
    TResult? Function()? noBlender,
    TResult? Function(String reason)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeBlenderState_Converting() when converting != null:
        return converting();
      case BridgeBlenderState_NoBlender() when noBlender != null:
        return noBlender();
      case BridgeBlenderState_Failed() when failed != null:
        return failed(_that.reason);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeBlenderState_Converting extends BridgeBlenderState {
  const BridgeBlenderState_Converting() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeBlenderState_Converting);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeBlenderState.converting()';
  }
}

/// @nodoc

class BridgeBlenderState_NoBlender extends BridgeBlenderState {
  const BridgeBlenderState_NoBlender() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeBlenderState_NoBlender);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeBlenderState.noBlender()';
  }
}

/// @nodoc

class BridgeBlenderState_Failed extends BridgeBlenderState {
  const BridgeBlenderState_Failed({required this.reason}) : super._();

  final String reason;

  /// Create a copy of BridgeBlenderState
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeBlenderState_FailedCopyWith<BridgeBlenderState_Failed> get copyWith =>
      _$BridgeBlenderState_FailedCopyWithImpl<BridgeBlenderState_Failed>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeBlenderState_Failed &&
            (identical(other.reason, reason) || other.reason == reason));
  }

  @override
  int get hashCode => Object.hash(runtimeType, reason);

  @override
  String toString() {
    return 'BridgeBlenderState.failed(reason: $reason)';
  }
}

/// @nodoc
abstract mixin class $BridgeBlenderState_FailedCopyWith<$Res>
    implements $BridgeBlenderStateCopyWith<$Res> {
  factory $BridgeBlenderState_FailedCopyWith(BridgeBlenderState_Failed value,
          $Res Function(BridgeBlenderState_Failed) _then) =
      _$BridgeBlenderState_FailedCopyWithImpl;
  @useResult
  $Res call({String reason});
}

/// @nodoc
class _$BridgeBlenderState_FailedCopyWithImpl<$Res>
    implements $BridgeBlenderState_FailedCopyWith<$Res> {
  _$BridgeBlenderState_FailedCopyWithImpl(this._self, this._then);

  final BridgeBlenderState_Failed _self;
  final $Res Function(BridgeBlenderState_Failed) _then;

  /// Create a copy of BridgeBlenderState
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? reason = null,
  }) {
    return _then(BridgeBlenderState_Failed(
      reason: null == reason
          ? _self.reason
          : reason // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

// dart format on
