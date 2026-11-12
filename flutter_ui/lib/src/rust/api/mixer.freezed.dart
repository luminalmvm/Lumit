// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'mixer.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$BridgeMixTarget {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeMixTarget);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeMixTarget()';
  }
}

/// @nodoc
class $BridgeMixTargetCopyWith<$Res> {
  $BridgeMixTargetCopyWith(
      BridgeMixTarget _, $Res Function(BridgeMixTarget) __);
}

/// Adds pattern-matching-related methods to [BridgeMixTarget].
extension BridgeMixTargetPatterns on BridgeMixTarget {
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
    TResult Function(BridgeMixTarget_Volume value)? volume,
    TResult Function(BridgeMixTarget_Pan value)? pan,
    TResult Function(BridgeMixTarget_Send value)? send,
    TResult Function(BridgeMixTarget_Master value)? master,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeMixTarget_Volume() when volume != null:
        return volume(_that);
      case BridgeMixTarget_Pan() when pan != null:
        return pan(_that);
      case BridgeMixTarget_Send() when send != null:
        return send(_that);
      case BridgeMixTarget_Master() when master != null:
        return master(_that);
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
    required TResult Function(BridgeMixTarget_Volume value) volume,
    required TResult Function(BridgeMixTarget_Pan value) pan,
    required TResult Function(BridgeMixTarget_Send value) send,
    required TResult Function(BridgeMixTarget_Master value) master,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeMixTarget_Volume():
        return volume(_that);
      case BridgeMixTarget_Pan():
        return pan(_that);
      case BridgeMixTarget_Send():
        return send(_that);
      case BridgeMixTarget_Master():
        return master(_that);
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
    TResult? Function(BridgeMixTarget_Volume value)? volume,
    TResult? Function(BridgeMixTarget_Pan value)? pan,
    TResult? Function(BridgeMixTarget_Send value)? send,
    TResult? Function(BridgeMixTarget_Master value)? master,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeMixTarget_Volume() when volume != null:
        return volume(_that);
      case BridgeMixTarget_Pan() when pan != null:
        return pan(_that);
      case BridgeMixTarget_Send() when send != null:
        return send(_that);
      case BridgeMixTarget_Master() when master != null:
        return master(_that);
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
    TResult Function(UuidValue layer)? volume,
    TResult Function(UuidValue layer)? pan,
    TResult Function(UuidValue layer, UuidValue bus)? send,
    TResult Function()? master,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeMixTarget_Volume() when volume != null:
        return volume(_that.layer);
      case BridgeMixTarget_Pan() when pan != null:
        return pan(_that.layer);
      case BridgeMixTarget_Send() when send != null:
        return send(_that.layer, _that.bus);
      case BridgeMixTarget_Master() when master != null:
        return master();
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
    required TResult Function(UuidValue layer) volume,
    required TResult Function(UuidValue layer) pan,
    required TResult Function(UuidValue layer, UuidValue bus) send,
    required TResult Function() master,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeMixTarget_Volume():
        return volume(_that.layer);
      case BridgeMixTarget_Pan():
        return pan(_that.layer);
      case BridgeMixTarget_Send():
        return send(_that.layer, _that.bus);
      case BridgeMixTarget_Master():
        return master();
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
    TResult? Function(UuidValue layer)? volume,
    TResult? Function(UuidValue layer)? pan,
    TResult? Function(UuidValue layer, UuidValue bus)? send,
    TResult? Function()? master,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeMixTarget_Volume() when volume != null:
        return volume(_that.layer);
      case BridgeMixTarget_Pan() when pan != null:
        return pan(_that.layer);
      case BridgeMixTarget_Send() when send != null:
        return send(_that.layer, _that.bus);
      case BridgeMixTarget_Master() when master != null:
        return master();
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeMixTarget_Volume extends BridgeMixTarget {
  const BridgeMixTarget_Volume({required this.layer}) : super._();

  final UuidValue layer;

  /// Create a copy of BridgeMixTarget
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeMixTarget_VolumeCopyWith<BridgeMixTarget_Volume> get copyWith =>
      _$BridgeMixTarget_VolumeCopyWithImpl<BridgeMixTarget_Volume>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeMixTarget_Volume &&
            (identical(other.layer, layer) || other.layer == layer));
  }

  @override
  int get hashCode => Object.hash(runtimeType, layer);

  @override
  String toString() {
    return 'BridgeMixTarget.volume(layer: $layer)';
  }
}

/// @nodoc
abstract mixin class $BridgeMixTarget_VolumeCopyWith<$Res>
    implements $BridgeMixTargetCopyWith<$Res> {
  factory $BridgeMixTarget_VolumeCopyWith(BridgeMixTarget_Volume value,
          $Res Function(BridgeMixTarget_Volume) _then) =
      _$BridgeMixTarget_VolumeCopyWithImpl;
  @useResult
  $Res call({UuidValue layer});
}

/// @nodoc
class _$BridgeMixTarget_VolumeCopyWithImpl<$Res>
    implements $BridgeMixTarget_VolumeCopyWith<$Res> {
  _$BridgeMixTarget_VolumeCopyWithImpl(this._self, this._then);

  final BridgeMixTarget_Volume _self;
  final $Res Function(BridgeMixTarget_Volume) _then;

  /// Create a copy of BridgeMixTarget
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? layer = null,
  }) {
    return _then(BridgeMixTarget_Volume(
      layer: null == layer
          ? _self.layer
          : layer // ignore: cast_nullable_to_non_nullable
              as UuidValue,
    ));
  }
}

/// @nodoc

class BridgeMixTarget_Pan extends BridgeMixTarget {
  const BridgeMixTarget_Pan({required this.layer}) : super._();

  final UuidValue layer;

  /// Create a copy of BridgeMixTarget
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeMixTarget_PanCopyWith<BridgeMixTarget_Pan> get copyWith =>
      _$BridgeMixTarget_PanCopyWithImpl<BridgeMixTarget_Pan>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeMixTarget_Pan &&
            (identical(other.layer, layer) || other.layer == layer));
  }

  @override
  int get hashCode => Object.hash(runtimeType, layer);

  @override
  String toString() {
    return 'BridgeMixTarget.pan(layer: $layer)';
  }
}

/// @nodoc
abstract mixin class $BridgeMixTarget_PanCopyWith<$Res>
    implements $BridgeMixTargetCopyWith<$Res> {
  factory $BridgeMixTarget_PanCopyWith(
          BridgeMixTarget_Pan value, $Res Function(BridgeMixTarget_Pan) _then) =
      _$BridgeMixTarget_PanCopyWithImpl;
  @useResult
  $Res call({UuidValue layer});
}

/// @nodoc
class _$BridgeMixTarget_PanCopyWithImpl<$Res>
    implements $BridgeMixTarget_PanCopyWith<$Res> {
  _$BridgeMixTarget_PanCopyWithImpl(this._self, this._then);

  final BridgeMixTarget_Pan _self;
  final $Res Function(BridgeMixTarget_Pan) _then;

  /// Create a copy of BridgeMixTarget
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? layer = null,
  }) {
    return _then(BridgeMixTarget_Pan(
      layer: null == layer
          ? _self.layer
          : layer // ignore: cast_nullable_to_non_nullable
              as UuidValue,
    ));
  }
}

/// @nodoc

class BridgeMixTarget_Send extends BridgeMixTarget {
  const BridgeMixTarget_Send({required this.layer, required this.bus})
      : super._();

  final UuidValue layer;
  final UuidValue bus;

  /// Create a copy of BridgeMixTarget
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeMixTarget_SendCopyWith<BridgeMixTarget_Send> get copyWith =>
      _$BridgeMixTarget_SendCopyWithImpl<BridgeMixTarget_Send>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeMixTarget_Send &&
            (identical(other.layer, layer) || other.layer == layer) &&
            (identical(other.bus, bus) || other.bus == bus));
  }

  @override
  int get hashCode => Object.hash(runtimeType, layer, bus);

  @override
  String toString() {
    return 'BridgeMixTarget.send(layer: $layer, bus: $bus)';
  }
}

/// @nodoc
abstract mixin class $BridgeMixTarget_SendCopyWith<$Res>
    implements $BridgeMixTargetCopyWith<$Res> {
  factory $BridgeMixTarget_SendCopyWith(BridgeMixTarget_Send value,
          $Res Function(BridgeMixTarget_Send) _then) =
      _$BridgeMixTarget_SendCopyWithImpl;
  @useResult
  $Res call({UuidValue layer, UuidValue bus});
}

/// @nodoc
class _$BridgeMixTarget_SendCopyWithImpl<$Res>
    implements $BridgeMixTarget_SendCopyWith<$Res> {
  _$BridgeMixTarget_SendCopyWithImpl(this._self, this._then);

  final BridgeMixTarget_Send _self;
  final $Res Function(BridgeMixTarget_Send) _then;

  /// Create a copy of BridgeMixTarget
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? layer = null,
    Object? bus = null,
  }) {
    return _then(BridgeMixTarget_Send(
      layer: null == layer
          ? _self.layer
          : layer // ignore: cast_nullable_to_non_nullable
              as UuidValue,
      bus: null == bus
          ? _self.bus
          : bus // ignore: cast_nullable_to_non_nullable
              as UuidValue,
    ));
  }
}

/// @nodoc

class BridgeMixTarget_Master extends BridgeMixTarget {
  const BridgeMixTarget_Master() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeMixTarget_Master);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeMixTarget.master()';
  }
}

// dart format on
