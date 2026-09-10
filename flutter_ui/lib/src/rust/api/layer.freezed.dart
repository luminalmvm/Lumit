// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'layer.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$BridgeClipFadeShape {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeClipFadeShape);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeClipFadeShape()';
  }
}

/// @nodoc
class $BridgeClipFadeShapeCopyWith<$Res> {
  $BridgeClipFadeShapeCopyWith(
      BridgeClipFadeShape _, $Res Function(BridgeClipFadeShape) __);
}

/// Adds pattern-matching-related methods to [BridgeClipFadeShape].
extension BridgeClipFadeShapePatterns on BridgeClipFadeShape {
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
    TResult Function(BridgeClipFadeShape_Linear value)? linear,
    TResult Function(BridgeClipFadeShape_Fast value)? fast,
    TResult Function(BridgeClipFadeShape_Slow value)? slow,
    TResult Function(BridgeClipFadeShape_Smooth value)? smooth,
    TResult Function(BridgeClipFadeShape_Sharp value)? sharp,
    TResult Function(BridgeClipFadeShape_Custom value)? custom,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeClipFadeShape_Linear() when linear != null:
        return linear(_that);
      case BridgeClipFadeShape_Fast() when fast != null:
        return fast(_that);
      case BridgeClipFadeShape_Slow() when slow != null:
        return slow(_that);
      case BridgeClipFadeShape_Smooth() when smooth != null:
        return smooth(_that);
      case BridgeClipFadeShape_Sharp() when sharp != null:
        return sharp(_that);
      case BridgeClipFadeShape_Custom() when custom != null:
        return custom(_that);
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
    required TResult Function(BridgeClipFadeShape_Linear value) linear,
    required TResult Function(BridgeClipFadeShape_Fast value) fast,
    required TResult Function(BridgeClipFadeShape_Slow value) slow,
    required TResult Function(BridgeClipFadeShape_Smooth value) smooth,
    required TResult Function(BridgeClipFadeShape_Sharp value) sharp,
    required TResult Function(BridgeClipFadeShape_Custom value) custom,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeClipFadeShape_Linear():
        return linear(_that);
      case BridgeClipFadeShape_Fast():
        return fast(_that);
      case BridgeClipFadeShape_Slow():
        return slow(_that);
      case BridgeClipFadeShape_Smooth():
        return smooth(_that);
      case BridgeClipFadeShape_Sharp():
        return sharp(_that);
      case BridgeClipFadeShape_Custom():
        return custom(_that);
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
    TResult? Function(BridgeClipFadeShape_Linear value)? linear,
    TResult? Function(BridgeClipFadeShape_Fast value)? fast,
    TResult? Function(BridgeClipFadeShape_Slow value)? slow,
    TResult? Function(BridgeClipFadeShape_Smooth value)? smooth,
    TResult? Function(BridgeClipFadeShape_Sharp value)? sharp,
    TResult? Function(BridgeClipFadeShape_Custom value)? custom,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeClipFadeShape_Linear() when linear != null:
        return linear(_that);
      case BridgeClipFadeShape_Fast() when fast != null:
        return fast(_that);
      case BridgeClipFadeShape_Slow() when slow != null:
        return slow(_that);
      case BridgeClipFadeShape_Smooth() when smooth != null:
        return smooth(_that);
      case BridgeClipFadeShape_Sharp() when sharp != null:
        return sharp(_that);
      case BridgeClipFadeShape_Custom() when custom != null:
        return custom(_that);
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
    TResult Function()? linear,
    TResult Function()? fast,
    TResult Function()? slow,
    TResult Function()? smooth,
    TResult Function()? sharp,
    TResult Function(double x1, double y1, double x2, double y2)? custom,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeClipFadeShape_Linear() when linear != null:
        return linear();
      case BridgeClipFadeShape_Fast() when fast != null:
        return fast();
      case BridgeClipFadeShape_Slow() when slow != null:
        return slow();
      case BridgeClipFadeShape_Smooth() when smooth != null:
        return smooth();
      case BridgeClipFadeShape_Sharp() when sharp != null:
        return sharp();
      case BridgeClipFadeShape_Custom() when custom != null:
        return custom(_that.x1, _that.y1, _that.x2, _that.y2);
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
    required TResult Function() linear,
    required TResult Function() fast,
    required TResult Function() slow,
    required TResult Function() smooth,
    required TResult Function() sharp,
    required TResult Function(double x1, double y1, double x2, double y2)
        custom,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeClipFadeShape_Linear():
        return linear();
      case BridgeClipFadeShape_Fast():
        return fast();
      case BridgeClipFadeShape_Slow():
        return slow();
      case BridgeClipFadeShape_Smooth():
        return smooth();
      case BridgeClipFadeShape_Sharp():
        return sharp();
      case BridgeClipFadeShape_Custom():
        return custom(_that.x1, _that.y1, _that.x2, _that.y2);
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
    TResult? Function()? linear,
    TResult? Function()? fast,
    TResult? Function()? slow,
    TResult? Function()? smooth,
    TResult? Function()? sharp,
    TResult? Function(double x1, double y1, double x2, double y2)? custom,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeClipFadeShape_Linear() when linear != null:
        return linear();
      case BridgeClipFadeShape_Fast() when fast != null:
        return fast();
      case BridgeClipFadeShape_Slow() when slow != null:
        return slow();
      case BridgeClipFadeShape_Smooth() when smooth != null:
        return smooth();
      case BridgeClipFadeShape_Sharp() when sharp != null:
        return sharp();
      case BridgeClipFadeShape_Custom() when custom != null:
        return custom(_that.x1, _that.y1, _that.x2, _that.y2);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeClipFadeShape_Linear extends BridgeClipFadeShape {
  const BridgeClipFadeShape_Linear() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeClipFadeShape_Linear);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeClipFadeShape.linear()';
  }
}

/// @nodoc

class BridgeClipFadeShape_Fast extends BridgeClipFadeShape {
  const BridgeClipFadeShape_Fast() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeClipFadeShape_Fast);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeClipFadeShape.fast()';
  }
}

/// @nodoc

class BridgeClipFadeShape_Slow extends BridgeClipFadeShape {
  const BridgeClipFadeShape_Slow() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeClipFadeShape_Slow);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeClipFadeShape.slow()';
  }
}

/// @nodoc

class BridgeClipFadeShape_Smooth extends BridgeClipFadeShape {
  const BridgeClipFadeShape_Smooth() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeClipFadeShape_Smooth);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeClipFadeShape.smooth()';
  }
}

/// @nodoc

class BridgeClipFadeShape_Sharp extends BridgeClipFadeShape {
  const BridgeClipFadeShape_Sharp() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeClipFadeShape_Sharp);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeClipFadeShape.sharp()';
  }
}

/// @nodoc

class BridgeClipFadeShape_Custom extends BridgeClipFadeShape {
  const BridgeClipFadeShape_Custom(
      {required this.x1, required this.y1, required this.x2, required this.y2})
      : super._();

  final double x1;
  final double y1;
  final double x2;
  final double y2;

  /// Create a copy of BridgeClipFadeShape
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeClipFadeShape_CustomCopyWith<BridgeClipFadeShape_Custom>
      get copyWith =>
          _$BridgeClipFadeShape_CustomCopyWithImpl<BridgeClipFadeShape_Custom>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeClipFadeShape_Custom &&
            (identical(other.x1, x1) || other.x1 == x1) &&
            (identical(other.y1, y1) || other.y1 == y1) &&
            (identical(other.x2, x2) || other.x2 == x2) &&
            (identical(other.y2, y2) || other.y2 == y2));
  }

  @override
  int get hashCode => Object.hash(runtimeType, x1, y1, x2, y2);

  @override
  String toString() {
    return 'BridgeClipFadeShape.custom(x1: $x1, y1: $y1, x2: $x2, y2: $y2)';
  }
}

/// @nodoc
abstract mixin class $BridgeClipFadeShape_CustomCopyWith<$Res>
    implements $BridgeClipFadeShapeCopyWith<$Res> {
  factory $BridgeClipFadeShape_CustomCopyWith(BridgeClipFadeShape_Custom value,
          $Res Function(BridgeClipFadeShape_Custom) _then) =
      _$BridgeClipFadeShape_CustomCopyWithImpl;
  @useResult
  $Res call({double x1, double y1, double x2, double y2});
}

/// @nodoc
class _$BridgeClipFadeShape_CustomCopyWithImpl<$Res>
    implements $BridgeClipFadeShape_CustomCopyWith<$Res> {
  _$BridgeClipFadeShape_CustomCopyWithImpl(this._self, this._then);

  final BridgeClipFadeShape_Custom _self;
  final $Res Function(BridgeClipFadeShape_Custom) _then;

  /// Create a copy of BridgeClipFadeShape
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? x1 = null,
    Object? y1 = null,
    Object? x2 = null,
    Object? y2 = null,
  }) {
    return _then(BridgeClipFadeShape_Custom(
      x1: null == x1
          ? _self.x1
          : x1 // ignore: cast_nullable_to_non_nullable
              as double,
      y1: null == y1
          ? _self.y1
          : y1 // ignore: cast_nullable_to_non_nullable
              as double,
      x2: null == x2
          ? _self.x2
          : x2 // ignore: cast_nullable_to_non_nullable
              as double,
      y2: null == y2
          ? _self.y2
          : y2 // ignore: cast_nullable_to_non_nullable
              as double,
    ));
  }
}

// dart format on
