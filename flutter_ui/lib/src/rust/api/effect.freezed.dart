// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'effect.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$BridgeEffectValue {
  Object? get field0;

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeEffectValue &&
            const DeepCollectionEquality().equals(other.field0, field0));
  }

  @override
  int get hashCode =>
      Object.hash(runtimeType, const DeepCollectionEquality().hash(field0));

  @override
  String toString() {
    return 'BridgeEffectValue(field0: $field0)';
  }
}

/// @nodoc
class $BridgeEffectValueCopyWith<$Res> {
  $BridgeEffectValueCopyWith(
      BridgeEffectValue _, $Res Function(BridgeEffectValue) __);
}

/// Adds pattern-matching-related methods to [BridgeEffectValue].
extension BridgeEffectValuePatterns on BridgeEffectValue {
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
    TResult Function(BridgeEffectValue_Float value)? float,
    TResult Function(BridgeEffectValue_Point value)? point,
    TResult Function(BridgeEffectValue_Colour value)? colour,
    TResult Function(BridgeEffectValue_Bool value)? bool,
    TResult Function(BridgeEffectValue_Choice value)? choice,
    TResult Function(BridgeEffectValue_Seed value)? seed,
    TResult Function(BridgeEffectValue_File value)? file,
    TResult Function(BridgeEffectValue_Layer value)? layer,
    TResult Function(BridgeEffectValue_MaskPath value)? maskPath,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeEffectValue_Float() when float != null:
        return float(_that);
      case BridgeEffectValue_Point() when point != null:
        return point(_that);
      case BridgeEffectValue_Colour() when colour != null:
        return colour(_that);
      case BridgeEffectValue_Bool() when bool != null:
        return bool(_that);
      case BridgeEffectValue_Choice() when choice != null:
        return choice(_that);
      case BridgeEffectValue_Seed() when seed != null:
        return seed(_that);
      case BridgeEffectValue_File() when file != null:
        return file(_that);
      case BridgeEffectValue_Layer() when layer != null:
        return layer(_that);
      case BridgeEffectValue_MaskPath() when maskPath != null:
        return maskPath(_that);
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
    required TResult Function(BridgeEffectValue_Float value) float,
    required TResult Function(BridgeEffectValue_Point value) point,
    required TResult Function(BridgeEffectValue_Colour value) colour,
    required TResult Function(BridgeEffectValue_Bool value) bool,
    required TResult Function(BridgeEffectValue_Choice value) choice,
    required TResult Function(BridgeEffectValue_Seed value) seed,
    required TResult Function(BridgeEffectValue_File value) file,
    required TResult Function(BridgeEffectValue_Layer value) layer,
    required TResult Function(BridgeEffectValue_MaskPath value) maskPath,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeEffectValue_Float():
        return float(_that);
      case BridgeEffectValue_Point():
        return point(_that);
      case BridgeEffectValue_Colour():
        return colour(_that);
      case BridgeEffectValue_Bool():
        return bool(_that);
      case BridgeEffectValue_Choice():
        return choice(_that);
      case BridgeEffectValue_Seed():
        return seed(_that);
      case BridgeEffectValue_File():
        return file(_that);
      case BridgeEffectValue_Layer():
        return layer(_that);
      case BridgeEffectValue_MaskPath():
        return maskPath(_that);
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
    TResult? Function(BridgeEffectValue_Float value)? float,
    TResult? Function(BridgeEffectValue_Point value)? point,
    TResult? Function(BridgeEffectValue_Colour value)? colour,
    TResult? Function(BridgeEffectValue_Bool value)? bool,
    TResult? Function(BridgeEffectValue_Choice value)? choice,
    TResult? Function(BridgeEffectValue_Seed value)? seed,
    TResult? Function(BridgeEffectValue_File value)? file,
    TResult? Function(BridgeEffectValue_Layer value)? layer,
    TResult? Function(BridgeEffectValue_MaskPath value)? maskPath,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeEffectValue_Float() when float != null:
        return float(_that);
      case BridgeEffectValue_Point() when point != null:
        return point(_that);
      case BridgeEffectValue_Colour() when colour != null:
        return colour(_that);
      case BridgeEffectValue_Bool() when bool != null:
        return bool(_that);
      case BridgeEffectValue_Choice() when choice != null:
        return choice(_that);
      case BridgeEffectValue_Seed() when seed != null:
        return seed(_that);
      case BridgeEffectValue_File() when file != null:
        return file(_that);
      case BridgeEffectValue_Layer() when layer != null:
        return layer(_that);
      case BridgeEffectValue_MaskPath() when maskPath != null:
        return maskPath(_that);
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
    TResult Function(BridgeScalar field0)? float,
    TResult Function(BridgePoint field0)? point,
    TResult Function(BridgeColour field0)? colour,
    TResult Function(bool field0)? bool,
    TResult Function(int field0)? choice,
    TResult Function(int field0)? seed,
    TResult Function(BridgeFileParam field0)? file,
    TResult Function(UuidValue? field0)? layer,
    TResult Function(UuidValue? field0)? maskPath,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeEffectValue_Float() when float != null:
        return float(_that.field0);
      case BridgeEffectValue_Point() when point != null:
        return point(_that.field0);
      case BridgeEffectValue_Colour() when colour != null:
        return colour(_that.field0);
      case BridgeEffectValue_Bool() when bool != null:
        return bool(_that.field0);
      case BridgeEffectValue_Choice() when choice != null:
        return choice(_that.field0);
      case BridgeEffectValue_Seed() when seed != null:
        return seed(_that.field0);
      case BridgeEffectValue_File() when file != null:
        return file(_that.field0);
      case BridgeEffectValue_Layer() when layer != null:
        return layer(_that.field0);
      case BridgeEffectValue_MaskPath() when maskPath != null:
        return maskPath(_that.field0);
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
    required TResult Function(BridgeScalar field0) float,
    required TResult Function(BridgePoint field0) point,
    required TResult Function(BridgeColour field0) colour,
    required TResult Function(bool field0) bool,
    required TResult Function(int field0) choice,
    required TResult Function(int field0) seed,
    required TResult Function(BridgeFileParam field0) file,
    required TResult Function(UuidValue? field0) layer,
    required TResult Function(UuidValue? field0) maskPath,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeEffectValue_Float():
        return float(_that.field0);
      case BridgeEffectValue_Point():
        return point(_that.field0);
      case BridgeEffectValue_Colour():
        return colour(_that.field0);
      case BridgeEffectValue_Bool():
        return bool(_that.field0);
      case BridgeEffectValue_Choice():
        return choice(_that.field0);
      case BridgeEffectValue_Seed():
        return seed(_that.field0);
      case BridgeEffectValue_File():
        return file(_that.field0);
      case BridgeEffectValue_Layer():
        return layer(_that.field0);
      case BridgeEffectValue_MaskPath():
        return maskPath(_that.field0);
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
    TResult? Function(BridgeScalar field0)? float,
    TResult? Function(BridgePoint field0)? point,
    TResult? Function(BridgeColour field0)? colour,
    TResult? Function(bool field0)? bool,
    TResult? Function(int field0)? choice,
    TResult? Function(int field0)? seed,
    TResult? Function(BridgeFileParam field0)? file,
    TResult? Function(UuidValue? field0)? layer,
    TResult? Function(UuidValue? field0)? maskPath,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeEffectValue_Float() when float != null:
        return float(_that.field0);
      case BridgeEffectValue_Point() when point != null:
        return point(_that.field0);
      case BridgeEffectValue_Colour() when colour != null:
        return colour(_that.field0);
      case BridgeEffectValue_Bool() when bool != null:
        return bool(_that.field0);
      case BridgeEffectValue_Choice() when choice != null:
        return choice(_that.field0);
      case BridgeEffectValue_Seed() when seed != null:
        return seed(_that.field0);
      case BridgeEffectValue_File() when file != null:
        return file(_that.field0);
      case BridgeEffectValue_Layer() when layer != null:
        return layer(_that.field0);
      case BridgeEffectValue_MaskPath() when maskPath != null:
        return maskPath(_that.field0);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeEffectValue_Float extends BridgeEffectValue {
  const BridgeEffectValue_Float(this.field0) : super._();

  @override
  final BridgeScalar field0;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeEffectValue_FloatCopyWith<BridgeEffectValue_Float> get copyWith =>
      _$BridgeEffectValue_FloatCopyWithImpl<BridgeEffectValue_Float>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeEffectValue_Float &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeEffectValue.float(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeEffectValue_FloatCopyWith<$Res>
    implements $BridgeEffectValueCopyWith<$Res> {
  factory $BridgeEffectValue_FloatCopyWith(BridgeEffectValue_Float value,
          $Res Function(BridgeEffectValue_Float) _then) =
      _$BridgeEffectValue_FloatCopyWithImpl;
  @useResult
  $Res call({BridgeScalar field0});

  $BridgeScalarCopyWith<$Res> get field0;
}

/// @nodoc
class _$BridgeEffectValue_FloatCopyWithImpl<$Res>
    implements $BridgeEffectValue_FloatCopyWith<$Res> {
  _$BridgeEffectValue_FloatCopyWithImpl(this._self, this._then);

  final BridgeEffectValue_Float _self;
  final $Res Function(BridgeEffectValue_Float) _then;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeEffectValue_Float(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeScalar,
    ));
  }

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @override
  @pragma('vm:prefer-inline')
  $BridgeScalarCopyWith<$Res> get field0 {
    return $BridgeScalarCopyWith<$Res>(_self.field0, (value) {
      return _then(_self.copyWith(field0: value));
    });
  }
}

/// @nodoc

class BridgeEffectValue_Point extends BridgeEffectValue {
  const BridgeEffectValue_Point(this.field0) : super._();

  @override
  final BridgePoint field0;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeEffectValue_PointCopyWith<BridgeEffectValue_Point> get copyWith =>
      _$BridgeEffectValue_PointCopyWithImpl<BridgeEffectValue_Point>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeEffectValue_Point &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeEffectValue.point(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeEffectValue_PointCopyWith<$Res>
    implements $BridgeEffectValueCopyWith<$Res> {
  factory $BridgeEffectValue_PointCopyWith(BridgeEffectValue_Point value,
          $Res Function(BridgeEffectValue_Point) _then) =
      _$BridgeEffectValue_PointCopyWithImpl;
  @useResult
  $Res call({BridgePoint field0});
}

/// @nodoc
class _$BridgeEffectValue_PointCopyWithImpl<$Res>
    implements $BridgeEffectValue_PointCopyWith<$Res> {
  _$BridgeEffectValue_PointCopyWithImpl(this._self, this._then);

  final BridgeEffectValue_Point _self;
  final $Res Function(BridgeEffectValue_Point) _then;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeEffectValue_Point(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgePoint,
    ));
  }
}

/// @nodoc

class BridgeEffectValue_Colour extends BridgeEffectValue {
  const BridgeEffectValue_Colour(this.field0) : super._();

  @override
  final BridgeColour field0;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeEffectValue_ColourCopyWith<BridgeEffectValue_Colour> get copyWith =>
      _$BridgeEffectValue_ColourCopyWithImpl<BridgeEffectValue_Colour>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeEffectValue_Colour &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeEffectValue.colour(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeEffectValue_ColourCopyWith<$Res>
    implements $BridgeEffectValueCopyWith<$Res> {
  factory $BridgeEffectValue_ColourCopyWith(BridgeEffectValue_Colour value,
          $Res Function(BridgeEffectValue_Colour) _then) =
      _$BridgeEffectValue_ColourCopyWithImpl;
  @useResult
  $Res call({BridgeColour field0});
}

/// @nodoc
class _$BridgeEffectValue_ColourCopyWithImpl<$Res>
    implements $BridgeEffectValue_ColourCopyWith<$Res> {
  _$BridgeEffectValue_ColourCopyWithImpl(this._self, this._then);

  final BridgeEffectValue_Colour _self;
  final $Res Function(BridgeEffectValue_Colour) _then;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeEffectValue_Colour(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeColour,
    ));
  }
}

/// @nodoc

class BridgeEffectValue_Bool extends BridgeEffectValue {
  const BridgeEffectValue_Bool(this.field0) : super._();

  @override
  final bool field0;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeEffectValue_BoolCopyWith<BridgeEffectValue_Bool> get copyWith =>
      _$BridgeEffectValue_BoolCopyWithImpl<BridgeEffectValue_Bool>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeEffectValue_Bool &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeEffectValue.bool(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeEffectValue_BoolCopyWith<$Res>
    implements $BridgeEffectValueCopyWith<$Res> {
  factory $BridgeEffectValue_BoolCopyWith(BridgeEffectValue_Bool value,
          $Res Function(BridgeEffectValue_Bool) _then) =
      _$BridgeEffectValue_BoolCopyWithImpl;
  @useResult
  $Res call({bool field0});
}

/// @nodoc
class _$BridgeEffectValue_BoolCopyWithImpl<$Res>
    implements $BridgeEffectValue_BoolCopyWith<$Res> {
  _$BridgeEffectValue_BoolCopyWithImpl(this._self, this._then);

  final BridgeEffectValue_Bool _self;
  final $Res Function(BridgeEffectValue_Bool) _then;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeEffectValue_Bool(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as bool,
    ));
  }
}

/// @nodoc

class BridgeEffectValue_Choice extends BridgeEffectValue {
  const BridgeEffectValue_Choice(this.field0) : super._();

  @override
  final int field0;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeEffectValue_ChoiceCopyWith<BridgeEffectValue_Choice> get copyWith =>
      _$BridgeEffectValue_ChoiceCopyWithImpl<BridgeEffectValue_Choice>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeEffectValue_Choice &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeEffectValue.choice(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeEffectValue_ChoiceCopyWith<$Res>
    implements $BridgeEffectValueCopyWith<$Res> {
  factory $BridgeEffectValue_ChoiceCopyWith(BridgeEffectValue_Choice value,
          $Res Function(BridgeEffectValue_Choice) _then) =
      _$BridgeEffectValue_ChoiceCopyWithImpl;
  @useResult
  $Res call({int field0});
}

/// @nodoc
class _$BridgeEffectValue_ChoiceCopyWithImpl<$Res>
    implements $BridgeEffectValue_ChoiceCopyWith<$Res> {
  _$BridgeEffectValue_ChoiceCopyWithImpl(this._self, this._then);

  final BridgeEffectValue_Choice _self;
  final $Res Function(BridgeEffectValue_Choice) _then;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeEffectValue_Choice(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as int,
    ));
  }
}

/// @nodoc

class BridgeEffectValue_Seed extends BridgeEffectValue {
  const BridgeEffectValue_Seed(this.field0) : super._();

  @override
  final int field0;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeEffectValue_SeedCopyWith<BridgeEffectValue_Seed> get copyWith =>
      _$BridgeEffectValue_SeedCopyWithImpl<BridgeEffectValue_Seed>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeEffectValue_Seed &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeEffectValue.seed(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeEffectValue_SeedCopyWith<$Res>
    implements $BridgeEffectValueCopyWith<$Res> {
  factory $BridgeEffectValue_SeedCopyWith(BridgeEffectValue_Seed value,
          $Res Function(BridgeEffectValue_Seed) _then) =
      _$BridgeEffectValue_SeedCopyWithImpl;
  @useResult
  $Res call({int field0});
}

/// @nodoc
class _$BridgeEffectValue_SeedCopyWithImpl<$Res>
    implements $BridgeEffectValue_SeedCopyWith<$Res> {
  _$BridgeEffectValue_SeedCopyWithImpl(this._self, this._then);

  final BridgeEffectValue_Seed _self;
  final $Res Function(BridgeEffectValue_Seed) _then;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeEffectValue_Seed(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as int,
    ));
  }
}

/// @nodoc

class BridgeEffectValue_File extends BridgeEffectValue {
  const BridgeEffectValue_File(this.field0) : super._();

  @override
  final BridgeFileParam field0;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeEffectValue_FileCopyWith<BridgeEffectValue_File> get copyWith =>
      _$BridgeEffectValue_FileCopyWithImpl<BridgeEffectValue_File>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeEffectValue_File &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeEffectValue.file(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeEffectValue_FileCopyWith<$Res>
    implements $BridgeEffectValueCopyWith<$Res> {
  factory $BridgeEffectValue_FileCopyWith(BridgeEffectValue_File value,
          $Res Function(BridgeEffectValue_File) _then) =
      _$BridgeEffectValue_FileCopyWithImpl;
  @useResult
  $Res call({BridgeFileParam field0});
}

/// @nodoc
class _$BridgeEffectValue_FileCopyWithImpl<$Res>
    implements $BridgeEffectValue_FileCopyWith<$Res> {
  _$BridgeEffectValue_FileCopyWithImpl(this._self, this._then);

  final BridgeEffectValue_File _self;
  final $Res Function(BridgeEffectValue_File) _then;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeEffectValue_File(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeFileParam,
    ));
  }
}

/// @nodoc

class BridgeEffectValue_Layer extends BridgeEffectValue {
  const BridgeEffectValue_Layer([this.field0]) : super._();

  @override
  final UuidValue? field0;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeEffectValue_LayerCopyWith<BridgeEffectValue_Layer> get copyWith =>
      _$BridgeEffectValue_LayerCopyWithImpl<BridgeEffectValue_Layer>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeEffectValue_Layer &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeEffectValue.layer(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeEffectValue_LayerCopyWith<$Res>
    implements $BridgeEffectValueCopyWith<$Res> {
  factory $BridgeEffectValue_LayerCopyWith(BridgeEffectValue_Layer value,
          $Res Function(BridgeEffectValue_Layer) _then) =
      _$BridgeEffectValue_LayerCopyWithImpl;
  @useResult
  $Res call({UuidValue? field0});
}

/// @nodoc
class _$BridgeEffectValue_LayerCopyWithImpl<$Res>
    implements $BridgeEffectValue_LayerCopyWith<$Res> {
  _$BridgeEffectValue_LayerCopyWithImpl(this._self, this._then);

  final BridgeEffectValue_Layer _self;
  final $Res Function(BridgeEffectValue_Layer) _then;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = freezed,
  }) {
    return _then(BridgeEffectValue_Layer(
      freezed == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as UuidValue?,
    ));
  }
}

/// @nodoc

class BridgeEffectValue_MaskPath extends BridgeEffectValue {
  const BridgeEffectValue_MaskPath([this.field0]) : super._();

  @override
  final UuidValue? field0;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeEffectValue_MaskPathCopyWith<BridgeEffectValue_MaskPath>
      get copyWith =>
          _$BridgeEffectValue_MaskPathCopyWithImpl<BridgeEffectValue_MaskPath>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeEffectValue_MaskPath &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeEffectValue.maskPath(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeEffectValue_MaskPathCopyWith<$Res>
    implements $BridgeEffectValueCopyWith<$Res> {
  factory $BridgeEffectValue_MaskPathCopyWith(BridgeEffectValue_MaskPath value,
          $Res Function(BridgeEffectValue_MaskPath) _then) =
      _$BridgeEffectValue_MaskPathCopyWithImpl;
  @useResult
  $Res call({UuidValue? field0});
}

/// @nodoc
class _$BridgeEffectValue_MaskPathCopyWithImpl<$Res>
    implements $BridgeEffectValue_MaskPathCopyWith<$Res> {
  _$BridgeEffectValue_MaskPathCopyWithImpl(this._self, this._then);

  final BridgeEffectValue_MaskPath _self;
  final $Res Function(BridgeEffectValue_MaskPath) _then;

  /// Create a copy of BridgeEffectValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = freezed,
  }) {
    return _then(BridgeEffectValue_MaskPath(
      freezed == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as UuidValue?,
    ));
  }
}

/// @nodoc
mixin _$BridgeEnabledCond {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeEnabledCond);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeEnabledCond()';
  }
}

/// @nodoc
class $BridgeEnabledCondCopyWith<$Res> {
  $BridgeEnabledCondCopyWith(
      BridgeEnabledCond _, $Res Function(BridgeEnabledCond) __);
}

/// Adds pattern-matching-related methods to [BridgeEnabledCond].
extension BridgeEnabledCondPatterns on BridgeEnabledCond {
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
    TResult Function(BridgeEnabledCond_BoolIs value)? boolIs,
    TResult Function(BridgeEnabledCond_ChoiceIs value)? choiceIs,
    TResult Function(BridgeEnabledCond_ChoiceIsNot value)? choiceIsNot,
    TResult Function(BridgeEnabledCond_LayerSet value)? layerSet,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeEnabledCond_BoolIs() when boolIs != null:
        return boolIs(_that);
      case BridgeEnabledCond_ChoiceIs() when choiceIs != null:
        return choiceIs(_that);
      case BridgeEnabledCond_ChoiceIsNot() when choiceIsNot != null:
        return choiceIsNot(_that);
      case BridgeEnabledCond_LayerSet() when layerSet != null:
        return layerSet(_that);
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
    required TResult Function(BridgeEnabledCond_BoolIs value) boolIs,
    required TResult Function(BridgeEnabledCond_ChoiceIs value) choiceIs,
    required TResult Function(BridgeEnabledCond_ChoiceIsNot value) choiceIsNot,
    required TResult Function(BridgeEnabledCond_LayerSet value) layerSet,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeEnabledCond_BoolIs():
        return boolIs(_that);
      case BridgeEnabledCond_ChoiceIs():
        return choiceIs(_that);
      case BridgeEnabledCond_ChoiceIsNot():
        return choiceIsNot(_that);
      case BridgeEnabledCond_LayerSet():
        return layerSet(_that);
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
    TResult? Function(BridgeEnabledCond_BoolIs value)? boolIs,
    TResult? Function(BridgeEnabledCond_ChoiceIs value)? choiceIs,
    TResult? Function(BridgeEnabledCond_ChoiceIsNot value)? choiceIsNot,
    TResult? Function(BridgeEnabledCond_LayerSet value)? layerSet,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeEnabledCond_BoolIs() when boolIs != null:
        return boolIs(_that);
      case BridgeEnabledCond_ChoiceIs() when choiceIs != null:
        return choiceIs(_that);
      case BridgeEnabledCond_ChoiceIsNot() when choiceIsNot != null:
        return choiceIsNot(_that);
      case BridgeEnabledCond_LayerSet() when layerSet != null:
        return layerSet(_that);
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
    TResult Function(bool field0)? boolIs,
    TResult Function(int field0)? choiceIs,
    TResult Function(int field0)? choiceIsNot,
    TResult Function()? layerSet,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeEnabledCond_BoolIs() when boolIs != null:
        return boolIs(_that.field0);
      case BridgeEnabledCond_ChoiceIs() when choiceIs != null:
        return choiceIs(_that.field0);
      case BridgeEnabledCond_ChoiceIsNot() when choiceIsNot != null:
        return choiceIsNot(_that.field0);
      case BridgeEnabledCond_LayerSet() when layerSet != null:
        return layerSet();
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
    required TResult Function(bool field0) boolIs,
    required TResult Function(int field0) choiceIs,
    required TResult Function(int field0) choiceIsNot,
    required TResult Function() layerSet,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeEnabledCond_BoolIs():
        return boolIs(_that.field0);
      case BridgeEnabledCond_ChoiceIs():
        return choiceIs(_that.field0);
      case BridgeEnabledCond_ChoiceIsNot():
        return choiceIsNot(_that.field0);
      case BridgeEnabledCond_LayerSet():
        return layerSet();
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
    TResult? Function(bool field0)? boolIs,
    TResult? Function(int field0)? choiceIs,
    TResult? Function(int field0)? choiceIsNot,
    TResult? Function()? layerSet,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeEnabledCond_BoolIs() when boolIs != null:
        return boolIs(_that.field0);
      case BridgeEnabledCond_ChoiceIs() when choiceIs != null:
        return choiceIs(_that.field0);
      case BridgeEnabledCond_ChoiceIsNot() when choiceIsNot != null:
        return choiceIsNot(_that.field0);
      case BridgeEnabledCond_LayerSet() when layerSet != null:
        return layerSet();
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeEnabledCond_BoolIs extends BridgeEnabledCond {
  const BridgeEnabledCond_BoolIs(this.field0) : super._();

  final bool field0;

  /// Create a copy of BridgeEnabledCond
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeEnabledCond_BoolIsCopyWith<BridgeEnabledCond_BoolIs> get copyWith =>
      _$BridgeEnabledCond_BoolIsCopyWithImpl<BridgeEnabledCond_BoolIs>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeEnabledCond_BoolIs &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeEnabledCond.boolIs(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeEnabledCond_BoolIsCopyWith<$Res>
    implements $BridgeEnabledCondCopyWith<$Res> {
  factory $BridgeEnabledCond_BoolIsCopyWith(BridgeEnabledCond_BoolIs value,
          $Res Function(BridgeEnabledCond_BoolIs) _then) =
      _$BridgeEnabledCond_BoolIsCopyWithImpl;
  @useResult
  $Res call({bool field0});
}

/// @nodoc
class _$BridgeEnabledCond_BoolIsCopyWithImpl<$Res>
    implements $BridgeEnabledCond_BoolIsCopyWith<$Res> {
  _$BridgeEnabledCond_BoolIsCopyWithImpl(this._self, this._then);

  final BridgeEnabledCond_BoolIs _self;
  final $Res Function(BridgeEnabledCond_BoolIs) _then;

  /// Create a copy of BridgeEnabledCond
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeEnabledCond_BoolIs(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as bool,
    ));
  }
}

/// @nodoc

class BridgeEnabledCond_ChoiceIs extends BridgeEnabledCond {
  const BridgeEnabledCond_ChoiceIs(this.field0) : super._();

  final int field0;

  /// Create a copy of BridgeEnabledCond
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeEnabledCond_ChoiceIsCopyWith<BridgeEnabledCond_ChoiceIs>
      get copyWith =>
          _$BridgeEnabledCond_ChoiceIsCopyWithImpl<BridgeEnabledCond_ChoiceIs>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeEnabledCond_ChoiceIs &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeEnabledCond.choiceIs(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeEnabledCond_ChoiceIsCopyWith<$Res>
    implements $BridgeEnabledCondCopyWith<$Res> {
  factory $BridgeEnabledCond_ChoiceIsCopyWith(BridgeEnabledCond_ChoiceIs value,
          $Res Function(BridgeEnabledCond_ChoiceIs) _then) =
      _$BridgeEnabledCond_ChoiceIsCopyWithImpl;
  @useResult
  $Res call({int field0});
}

/// @nodoc
class _$BridgeEnabledCond_ChoiceIsCopyWithImpl<$Res>
    implements $BridgeEnabledCond_ChoiceIsCopyWith<$Res> {
  _$BridgeEnabledCond_ChoiceIsCopyWithImpl(this._self, this._then);

  final BridgeEnabledCond_ChoiceIs _self;
  final $Res Function(BridgeEnabledCond_ChoiceIs) _then;

  /// Create a copy of BridgeEnabledCond
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeEnabledCond_ChoiceIs(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as int,
    ));
  }
}

/// @nodoc

class BridgeEnabledCond_ChoiceIsNot extends BridgeEnabledCond {
  const BridgeEnabledCond_ChoiceIsNot(this.field0) : super._();

  final int field0;

  /// Create a copy of BridgeEnabledCond
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeEnabledCond_ChoiceIsNotCopyWith<BridgeEnabledCond_ChoiceIsNot>
      get copyWith => _$BridgeEnabledCond_ChoiceIsNotCopyWithImpl<
          BridgeEnabledCond_ChoiceIsNot>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeEnabledCond_ChoiceIsNot &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeEnabledCond.choiceIsNot(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeEnabledCond_ChoiceIsNotCopyWith<$Res>
    implements $BridgeEnabledCondCopyWith<$Res> {
  factory $BridgeEnabledCond_ChoiceIsNotCopyWith(
          BridgeEnabledCond_ChoiceIsNot value,
          $Res Function(BridgeEnabledCond_ChoiceIsNot) _then) =
      _$BridgeEnabledCond_ChoiceIsNotCopyWithImpl;
  @useResult
  $Res call({int field0});
}

/// @nodoc
class _$BridgeEnabledCond_ChoiceIsNotCopyWithImpl<$Res>
    implements $BridgeEnabledCond_ChoiceIsNotCopyWith<$Res> {
  _$BridgeEnabledCond_ChoiceIsNotCopyWithImpl(this._self, this._then);

  final BridgeEnabledCond_ChoiceIsNot _self;
  final $Res Function(BridgeEnabledCond_ChoiceIsNot) _then;

  /// Create a copy of BridgeEnabledCond
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeEnabledCond_ChoiceIsNot(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as int,
    ));
  }
}

/// @nodoc

class BridgeEnabledCond_LayerSet extends BridgeEnabledCond {
  const BridgeEnabledCond_LayerSet() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeEnabledCond_LayerSet);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeEnabledCond.layerSet()';
  }
}

/// @nodoc
mixin _$BridgeParamKind {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeParamKind);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeParamKind()';
  }
}

/// @nodoc
class $BridgeParamKindCopyWith<$Res> {
  $BridgeParamKindCopyWith(
      BridgeParamKind _, $Res Function(BridgeParamKind) __);
}

/// Adds pattern-matching-related methods to [BridgeParamKind].
extension BridgeParamKindPatterns on BridgeParamKind {
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
    TResult Function(BridgeParamKind_Float value)? float,
    TResult Function(BridgeParamKind_Int value)? int,
    TResult Function(BridgeParamKind_Angle value)? angle,
    TResult Function(BridgeParamKind_Choice value)? choice,
    TResult Function(BridgeParamKind_Bool value)? bool,
    TResult Function(BridgeParamKind_Colour value)? colour,
    TResult Function(BridgeParamKind_Seed value)? seed,
    TResult Function(BridgeParamKind_File value)? file,
    TResult Function(BridgeParamKind_Layer value)? layer,
    TResult Function(BridgeParamKind_MaskPath value)? maskPath,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeParamKind_Float() when float != null:
        return float(_that);
      case BridgeParamKind_Int() when int != null:
        return int(_that);
      case BridgeParamKind_Angle() when angle != null:
        return angle(_that);
      case BridgeParamKind_Choice() when choice != null:
        return choice(_that);
      case BridgeParamKind_Bool() when bool != null:
        return bool(_that);
      case BridgeParamKind_Colour() when colour != null:
        return colour(_that);
      case BridgeParamKind_Seed() when seed != null:
        return seed(_that);
      case BridgeParamKind_File() when file != null:
        return file(_that);
      case BridgeParamKind_Layer() when layer != null:
        return layer(_that);
      case BridgeParamKind_MaskPath() when maskPath != null:
        return maskPath(_that);
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
    required TResult Function(BridgeParamKind_Float value) float,
    required TResult Function(BridgeParamKind_Int value) int,
    required TResult Function(BridgeParamKind_Angle value) angle,
    required TResult Function(BridgeParamKind_Choice value) choice,
    required TResult Function(BridgeParamKind_Bool value) bool,
    required TResult Function(BridgeParamKind_Colour value) colour,
    required TResult Function(BridgeParamKind_Seed value) seed,
    required TResult Function(BridgeParamKind_File value) file,
    required TResult Function(BridgeParamKind_Layer value) layer,
    required TResult Function(BridgeParamKind_MaskPath value) maskPath,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeParamKind_Float():
        return float(_that);
      case BridgeParamKind_Int():
        return int(_that);
      case BridgeParamKind_Angle():
        return angle(_that);
      case BridgeParamKind_Choice():
        return choice(_that);
      case BridgeParamKind_Bool():
        return bool(_that);
      case BridgeParamKind_Colour():
        return colour(_that);
      case BridgeParamKind_Seed():
        return seed(_that);
      case BridgeParamKind_File():
        return file(_that);
      case BridgeParamKind_Layer():
        return layer(_that);
      case BridgeParamKind_MaskPath():
        return maskPath(_that);
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
    TResult? Function(BridgeParamKind_Float value)? float,
    TResult? Function(BridgeParamKind_Int value)? int,
    TResult? Function(BridgeParamKind_Angle value)? angle,
    TResult? Function(BridgeParamKind_Choice value)? choice,
    TResult? Function(BridgeParamKind_Bool value)? bool,
    TResult? Function(BridgeParamKind_Colour value)? colour,
    TResult? Function(BridgeParamKind_Seed value)? seed,
    TResult? Function(BridgeParamKind_File value)? file,
    TResult? Function(BridgeParamKind_Layer value)? layer,
    TResult? Function(BridgeParamKind_MaskPath value)? maskPath,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeParamKind_Float() when float != null:
        return float(_that);
      case BridgeParamKind_Int() when int != null:
        return int(_that);
      case BridgeParamKind_Angle() when angle != null:
        return angle(_that);
      case BridgeParamKind_Choice() when choice != null:
        return choice(_that);
      case BridgeParamKind_Bool() when bool != null:
        return bool(_that);
      case BridgeParamKind_Colour() when colour != null:
        return colour(_that);
      case BridgeParamKind_Seed() when seed != null:
        return seed(_that);
      case BridgeParamKind_File() when file != null:
        return file(_that);
      case BridgeParamKind_Layer() when layer != null:
        return layer(_that);
      case BridgeParamKind_MaskPath() when maskPath != null:
        return maskPath(_that);
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
    TResult Function(double default_, double sliderMin, double sliderMax,
            double? hardMin, double? hardMax)?
        float,
    TResult Function(
            PlatformInt64 default_,
            PlatformInt64 sliderMin,
            PlatformInt64 sliderMax,
            PlatformInt64? hardMin,
            PlatformInt64? hardMax)?
        int,
    TResult Function(double default_, double dialStep)? angle,
    TResult Function(
            List<String> options, int default_, Uint32List dividersAfter)?
        choice,
    TResult Function(bool default_)? bool,
    TResult Function(Float64List default_, double min, double max)? colour,
    TResult Function()? seed,
    TResult Function(List<String> filter, String filterName)? file,
    TResult Function()? layer,
    TResult Function()? maskPath,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeParamKind_Float() when float != null:
        return float(_that.default_, _that.sliderMin, _that.sliderMax,
            _that.hardMin, _that.hardMax);
      case BridgeParamKind_Int() when int != null:
        return int(_that.default_, _that.sliderMin, _that.sliderMax,
            _that.hardMin, _that.hardMax);
      case BridgeParamKind_Angle() when angle != null:
        return angle(_that.default_, _that.dialStep);
      case BridgeParamKind_Choice() when choice != null:
        return choice(_that.options, _that.default_, _that.dividersAfter);
      case BridgeParamKind_Bool() when bool != null:
        return bool(_that.default_);
      case BridgeParamKind_Colour() when colour != null:
        return colour(_that.default_, _that.min, _that.max);
      case BridgeParamKind_Seed() when seed != null:
        return seed();
      case BridgeParamKind_File() when file != null:
        return file(_that.filter, _that.filterName);
      case BridgeParamKind_Layer() when layer != null:
        return layer();
      case BridgeParamKind_MaskPath() when maskPath != null:
        return maskPath();
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
    required TResult Function(double default_, double sliderMin,
            double sliderMax, double? hardMin, double? hardMax)
        float,
    required TResult Function(
            PlatformInt64 default_,
            PlatformInt64 sliderMin,
            PlatformInt64 sliderMax,
            PlatformInt64? hardMin,
            PlatformInt64? hardMax)
        int,
    required TResult Function(double default_, double dialStep) angle,
    required TResult Function(
            List<String> options, int default_, Uint32List dividersAfter)
        choice,
    required TResult Function(bool default_) bool,
    required TResult Function(Float64List default_, double min, double max)
        colour,
    required TResult Function() seed,
    required TResult Function(List<String> filter, String filterName) file,
    required TResult Function() layer,
    required TResult Function() maskPath,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeParamKind_Float():
        return float(_that.default_, _that.sliderMin, _that.sliderMax,
            _that.hardMin, _that.hardMax);
      case BridgeParamKind_Int():
        return int(_that.default_, _that.sliderMin, _that.sliderMax,
            _that.hardMin, _that.hardMax);
      case BridgeParamKind_Angle():
        return angle(_that.default_, _that.dialStep);
      case BridgeParamKind_Choice():
        return choice(_that.options, _that.default_, _that.dividersAfter);
      case BridgeParamKind_Bool():
        return bool(_that.default_);
      case BridgeParamKind_Colour():
        return colour(_that.default_, _that.min, _that.max);
      case BridgeParamKind_Seed():
        return seed();
      case BridgeParamKind_File():
        return file(_that.filter, _that.filterName);
      case BridgeParamKind_Layer():
        return layer();
      case BridgeParamKind_MaskPath():
        return maskPath();
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
    TResult? Function(double default_, double sliderMin, double sliderMax,
            double? hardMin, double? hardMax)?
        float,
    TResult? Function(
            PlatformInt64 default_,
            PlatformInt64 sliderMin,
            PlatformInt64 sliderMax,
            PlatformInt64? hardMin,
            PlatformInt64? hardMax)?
        int,
    TResult? Function(double default_, double dialStep)? angle,
    TResult? Function(
            List<String> options, int default_, Uint32List dividersAfter)?
        choice,
    TResult? Function(bool default_)? bool,
    TResult? Function(Float64List default_, double min, double max)? colour,
    TResult? Function()? seed,
    TResult? Function(List<String> filter, String filterName)? file,
    TResult? Function()? layer,
    TResult? Function()? maskPath,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeParamKind_Float() when float != null:
        return float(_that.default_, _that.sliderMin, _that.sliderMax,
            _that.hardMin, _that.hardMax);
      case BridgeParamKind_Int() when int != null:
        return int(_that.default_, _that.sliderMin, _that.sliderMax,
            _that.hardMin, _that.hardMax);
      case BridgeParamKind_Angle() when angle != null:
        return angle(_that.default_, _that.dialStep);
      case BridgeParamKind_Choice() when choice != null:
        return choice(_that.options, _that.default_, _that.dividersAfter);
      case BridgeParamKind_Bool() when bool != null:
        return bool(_that.default_);
      case BridgeParamKind_Colour() when colour != null:
        return colour(_that.default_, _that.min, _that.max);
      case BridgeParamKind_Seed() when seed != null:
        return seed();
      case BridgeParamKind_File() when file != null:
        return file(_that.filter, _that.filterName);
      case BridgeParamKind_Layer() when layer != null:
        return layer();
      case BridgeParamKind_MaskPath() when maskPath != null:
        return maskPath();
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeParamKind_Float extends BridgeParamKind {
  const BridgeParamKind_Float(
      {required this.default_,
      required this.sliderMin,
      required this.sliderMax,
      this.hardMin,
      this.hardMax})
      : super._();

  final double default_;

  /// The slider's travel. Typing may exceed it (docs/08 §1.2); only
  /// `hard_min`/`hard_max` may not.
  final double sliderMin;
  final double sliderMax;

  /// Hard bounds, either side open (K-090: a threshold clamps at zero
  /// below and runs unbounded above).
  final double? hardMin;
  final double? hardMax;

  /// Create a copy of BridgeParamKind
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeParamKind_FloatCopyWith<BridgeParamKind_Float> get copyWith =>
      _$BridgeParamKind_FloatCopyWithImpl<BridgeParamKind_Float>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeParamKind_Float &&
            (identical(other.default_, default_) ||
                other.default_ == default_) &&
            (identical(other.sliderMin, sliderMin) ||
                other.sliderMin == sliderMin) &&
            (identical(other.sliderMax, sliderMax) ||
                other.sliderMax == sliderMax) &&
            (identical(other.hardMin, hardMin) || other.hardMin == hardMin) &&
            (identical(other.hardMax, hardMax) || other.hardMax == hardMax));
  }

  @override
  int get hashCode => Object.hash(
      runtimeType, default_, sliderMin, sliderMax, hardMin, hardMax);

  @override
  String toString() {
    return 'BridgeParamKind.float(default_: $default_, sliderMin: $sliderMin, sliderMax: $sliderMax, hardMin: $hardMin, hardMax: $hardMax)';
  }
}

/// @nodoc
abstract mixin class $BridgeParamKind_FloatCopyWith<$Res>
    implements $BridgeParamKindCopyWith<$Res> {
  factory $BridgeParamKind_FloatCopyWith(BridgeParamKind_Float value,
          $Res Function(BridgeParamKind_Float) _then) =
      _$BridgeParamKind_FloatCopyWithImpl;
  @useResult
  $Res call(
      {double default_,
      double sliderMin,
      double sliderMax,
      double? hardMin,
      double? hardMax});
}

/// @nodoc
class _$BridgeParamKind_FloatCopyWithImpl<$Res>
    implements $BridgeParamKind_FloatCopyWith<$Res> {
  _$BridgeParamKind_FloatCopyWithImpl(this._self, this._then);

  final BridgeParamKind_Float _self;
  final $Res Function(BridgeParamKind_Float) _then;

  /// Create a copy of BridgeParamKind
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? default_ = null,
    Object? sliderMin = null,
    Object? sliderMax = null,
    Object? hardMin = freezed,
    Object? hardMax = freezed,
  }) {
    return _then(BridgeParamKind_Float(
      default_: null == default_
          ? _self.default_
          : default_ // ignore: cast_nullable_to_non_nullable
              as double,
      sliderMin: null == sliderMin
          ? _self.sliderMin
          : sliderMin // ignore: cast_nullable_to_non_nullable
              as double,
      sliderMax: null == sliderMax
          ? _self.sliderMax
          : sliderMax // ignore: cast_nullable_to_non_nullable
              as double,
      hardMin: freezed == hardMin
          ? _self.hardMin
          : hardMin // ignore: cast_nullable_to_non_nullable
              as double?,
      hardMax: freezed == hardMax
          ? _self.hardMax
          : hardMax // ignore: cast_nullable_to_non_nullable
              as double?,
    ));
  }
}

/// @nodoc

class BridgeParamKind_Int extends BridgeParamKind {
  const BridgeParamKind_Int(
      {required this.default_,
      required this.sliderMin,
      required this.sliderMax,
      this.hardMin,
      this.hardMax})
      : super._();

  final PlatformInt64 default_;
  final PlatformInt64 sliderMin;
  final PlatformInt64 sliderMax;
  final PlatformInt64? hardMin;
  final PlatformInt64? hardMax;

  /// Create a copy of BridgeParamKind
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeParamKind_IntCopyWith<BridgeParamKind_Int> get copyWith =>
      _$BridgeParamKind_IntCopyWithImpl<BridgeParamKind_Int>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeParamKind_Int &&
            (identical(other.default_, default_) ||
                other.default_ == default_) &&
            (identical(other.sliderMin, sliderMin) ||
                other.sliderMin == sliderMin) &&
            (identical(other.sliderMax, sliderMax) ||
                other.sliderMax == sliderMax) &&
            (identical(other.hardMin, hardMin) || other.hardMin == hardMin) &&
            (identical(other.hardMax, hardMax) || other.hardMax == hardMax));
  }

  @override
  int get hashCode => Object.hash(
      runtimeType, default_, sliderMin, sliderMax, hardMin, hardMax);

  @override
  String toString() {
    return 'BridgeParamKind.int(default_: $default_, sliderMin: $sliderMin, sliderMax: $sliderMax, hardMin: $hardMin, hardMax: $hardMax)';
  }
}

/// @nodoc
abstract mixin class $BridgeParamKind_IntCopyWith<$Res>
    implements $BridgeParamKindCopyWith<$Res> {
  factory $BridgeParamKind_IntCopyWith(
          BridgeParamKind_Int value, $Res Function(BridgeParamKind_Int) _then) =
      _$BridgeParamKind_IntCopyWithImpl;
  @useResult
  $Res call(
      {PlatformInt64 default_,
      PlatformInt64 sliderMin,
      PlatformInt64 sliderMax,
      PlatformInt64? hardMin,
      PlatformInt64? hardMax});
}

/// @nodoc
class _$BridgeParamKind_IntCopyWithImpl<$Res>
    implements $BridgeParamKind_IntCopyWith<$Res> {
  _$BridgeParamKind_IntCopyWithImpl(this._self, this._then);

  final BridgeParamKind_Int _self;
  final $Res Function(BridgeParamKind_Int) _then;

  /// Create a copy of BridgeParamKind
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? default_ = null,
    Object? sliderMin = null,
    Object? sliderMax = null,
    Object? hardMin = freezed,
    Object? hardMax = freezed,
  }) {
    return _then(BridgeParamKind_Int(
      default_: null == default_
          ? _self.default_
          : default_ // ignore: cast_nullable_to_non_nullable
              as PlatformInt64,
      sliderMin: null == sliderMin
          ? _self.sliderMin
          : sliderMin // ignore: cast_nullable_to_non_nullable
              as PlatformInt64,
      sliderMax: null == sliderMax
          ? _self.sliderMax
          : sliderMax // ignore: cast_nullable_to_non_nullable
              as PlatformInt64,
      hardMin: freezed == hardMin
          ? _self.hardMin
          : hardMin // ignore: cast_nullable_to_non_nullable
              as PlatformInt64?,
      hardMax: freezed == hardMax
          ? _self.hardMax
          : hardMax // ignore: cast_nullable_to_non_nullable
              as PlatformInt64?,
    ));
  }
}

/// @nodoc

class BridgeParamKind_Angle extends BridgeParamKind {
  const BridgeParamKind_Angle({required this.default_, required this.dialStep})
      : super._();

  final double default_;

  /// Snapping increment in degrees while a modifier is held.
  final double dialStep;

  /// Create a copy of BridgeParamKind
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeParamKind_AngleCopyWith<BridgeParamKind_Angle> get copyWith =>
      _$BridgeParamKind_AngleCopyWithImpl<BridgeParamKind_Angle>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeParamKind_Angle &&
            (identical(other.default_, default_) ||
                other.default_ == default_) &&
            (identical(other.dialStep, dialStep) ||
                other.dialStep == dialStep));
  }

  @override
  int get hashCode => Object.hash(runtimeType, default_, dialStep);

  @override
  String toString() {
    return 'BridgeParamKind.angle(default_: $default_, dialStep: $dialStep)';
  }
}

/// @nodoc
abstract mixin class $BridgeParamKind_AngleCopyWith<$Res>
    implements $BridgeParamKindCopyWith<$Res> {
  factory $BridgeParamKind_AngleCopyWith(BridgeParamKind_Angle value,
          $Res Function(BridgeParamKind_Angle) _then) =
      _$BridgeParamKind_AngleCopyWithImpl;
  @useResult
  $Res call({double default_, double dialStep});
}

/// @nodoc
class _$BridgeParamKind_AngleCopyWithImpl<$Res>
    implements $BridgeParamKind_AngleCopyWith<$Res> {
  _$BridgeParamKind_AngleCopyWithImpl(this._self, this._then);

  final BridgeParamKind_Angle _self;
  final $Res Function(BridgeParamKind_Angle) _then;

  /// Create a copy of BridgeParamKind
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? default_ = null,
    Object? dialStep = null,
  }) {
    return _then(BridgeParamKind_Angle(
      default_: null == default_
          ? _self.default_
          : default_ // ignore: cast_nullable_to_non_nullable
              as double,
      dialStep: null == dialStep
          ? _self.dialStep
          : dialStep // ignore: cast_nullable_to_non_nullable
              as double,
    ));
  }
}

/// @nodoc

class BridgeParamKind_Choice extends BridgeParamKind {
  const BridgeParamKind_Choice(
      {required final List<String> options,
      required this.default_,
      required this.dividersAfter})
      : _options = options,
        super._();

  final List<String> _options;
  List<String> get options {
    if (_options is EqualUnmodifiableListView) return _options;
    // ignore: implicit_dynamic_type
    return EqualUnmodifiableListView(_options);
  }

  final int default_;

  /// Option indices after which the dropdown draws a group divider (T21).
  /// Empty for an ungrouped list.
  final Uint32List dividersAfter;

  /// Create a copy of BridgeParamKind
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeParamKind_ChoiceCopyWith<BridgeParamKind_Choice> get copyWith =>
      _$BridgeParamKind_ChoiceCopyWithImpl<BridgeParamKind_Choice>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeParamKind_Choice &&
            const DeepCollectionEquality().equals(other._options, _options) &&
            (identical(other.default_, default_) ||
                other.default_ == default_) &&
            const DeepCollectionEquality()
                .equals(other.dividersAfter, dividersAfter));
  }

  @override
  int get hashCode => Object.hash(
      runtimeType,
      const DeepCollectionEquality().hash(_options),
      default_,
      const DeepCollectionEquality().hash(dividersAfter));

  @override
  String toString() {
    return 'BridgeParamKind.choice(options: $options, default_: $default_, dividersAfter: $dividersAfter)';
  }
}

/// @nodoc
abstract mixin class $BridgeParamKind_ChoiceCopyWith<$Res>
    implements $BridgeParamKindCopyWith<$Res> {
  factory $BridgeParamKind_ChoiceCopyWith(BridgeParamKind_Choice value,
          $Res Function(BridgeParamKind_Choice) _then) =
      _$BridgeParamKind_ChoiceCopyWithImpl;
  @useResult
  $Res call({List<String> options, int default_, Uint32List dividersAfter});
}

/// @nodoc
class _$BridgeParamKind_ChoiceCopyWithImpl<$Res>
    implements $BridgeParamKind_ChoiceCopyWith<$Res> {
  _$BridgeParamKind_ChoiceCopyWithImpl(this._self, this._then);

  final BridgeParamKind_Choice _self;
  final $Res Function(BridgeParamKind_Choice) _then;

  /// Create a copy of BridgeParamKind
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? options = null,
    Object? default_ = null,
    Object? dividersAfter = null,
  }) {
    return _then(BridgeParamKind_Choice(
      options: null == options
          ? _self._options
          : options // ignore: cast_nullable_to_non_nullable
              as List<String>,
      default_: null == default_
          ? _self.default_
          : default_ // ignore: cast_nullable_to_non_nullable
              as int,
      dividersAfter: null == dividersAfter
          ? _self.dividersAfter
          : dividersAfter // ignore: cast_nullable_to_non_nullable
              as Uint32List,
    ));
  }
}

/// @nodoc

class BridgeParamKind_Bool extends BridgeParamKind {
  const BridgeParamKind_Bool({required this.default_}) : super._();

  final bool default_;

  /// Create a copy of BridgeParamKind
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeParamKind_BoolCopyWith<BridgeParamKind_Bool> get copyWith =>
      _$BridgeParamKind_BoolCopyWithImpl<BridgeParamKind_Bool>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeParamKind_Bool &&
            (identical(other.default_, default_) ||
                other.default_ == default_));
  }

  @override
  int get hashCode => Object.hash(runtimeType, default_);

  @override
  String toString() {
    return 'BridgeParamKind.bool(default_: $default_)';
  }
}

/// @nodoc
abstract mixin class $BridgeParamKind_BoolCopyWith<$Res>
    implements $BridgeParamKindCopyWith<$Res> {
  factory $BridgeParamKind_BoolCopyWith(BridgeParamKind_Bool value,
          $Res Function(BridgeParamKind_Bool) _then) =
      _$BridgeParamKind_BoolCopyWithImpl;
  @useResult
  $Res call({bool default_});
}

/// @nodoc
class _$BridgeParamKind_BoolCopyWithImpl<$Res>
    implements $BridgeParamKind_BoolCopyWith<$Res> {
  _$BridgeParamKind_BoolCopyWithImpl(this._self, this._then);

  final BridgeParamKind_Bool _self;
  final $Res Function(BridgeParamKind_Bool) _then;

  /// Create a copy of BridgeParamKind
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? default_ = null,
  }) {
    return _then(BridgeParamKind_Bool(
      default_: null == default_
          ? _self.default_
          : default_ // ignore: cast_nullable_to_non_nullable
              as bool,
    ));
  }
}

/// @nodoc

class BridgeParamKind_Colour extends BridgeParamKind {
  const BridgeParamKind_Colour(
      {required this.default_, required this.min, required this.max})
      : super._();

  /// Scene-linear RGBA. Channels animate independently in the model, so
  /// the panel edits four scalars behind one swatch.
  final Float64List default_;

  /// Per-channel edit range — a linear value may exceed 1 (an HDR tint)
  /// or dip below 0 (a lift), so each colour declares its own.
  final double min;
  final double max;

  /// Create a copy of BridgeParamKind
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeParamKind_ColourCopyWith<BridgeParamKind_Colour> get copyWith =>
      _$BridgeParamKind_ColourCopyWithImpl<BridgeParamKind_Colour>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeParamKind_Colour &&
            const DeepCollectionEquality().equals(other.default_, default_) &&
            (identical(other.min, min) || other.min == min) &&
            (identical(other.max, max) || other.max == max));
  }

  @override
  int get hashCode => Object.hash(
      runtimeType, const DeepCollectionEquality().hash(default_), min, max);

  @override
  String toString() {
    return 'BridgeParamKind.colour(default_: $default_, min: $min, max: $max)';
  }
}

/// @nodoc
abstract mixin class $BridgeParamKind_ColourCopyWith<$Res>
    implements $BridgeParamKindCopyWith<$Res> {
  factory $BridgeParamKind_ColourCopyWith(BridgeParamKind_Colour value,
          $Res Function(BridgeParamKind_Colour) _then) =
      _$BridgeParamKind_ColourCopyWithImpl;
  @useResult
  $Res call({Float64List default_, double min, double max});
}

/// @nodoc
class _$BridgeParamKind_ColourCopyWithImpl<$Res>
    implements $BridgeParamKind_ColourCopyWith<$Res> {
  _$BridgeParamKind_ColourCopyWithImpl(this._self, this._then);

  final BridgeParamKind_Colour _self;
  final $Res Function(BridgeParamKind_Colour) _then;

  /// Create a copy of BridgeParamKind
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? default_ = null,
    Object? min = null,
    Object? max = null,
  }) {
    return _then(BridgeParamKind_Colour(
      default_: null == default_
          ? _self.default_
          : default_ // ignore: cast_nullable_to_non_nullable
              as Float64List,
      min: null == min
          ? _self.min
          : min // ignore: cast_nullable_to_non_nullable
              as double,
      max: null == max
          ? _self.max
          : max // ignore: cast_nullable_to_non_nullable
              as double,
    ));
  }
}

/// @nodoc

class BridgeParamKind_Seed extends BridgeParamKind {
  const BridgeParamKind_Seed() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeParamKind_Seed);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeParamKind.seed()';
  }
}

/// @nodoc

class BridgeParamKind_File extends BridgeParamKind {
  const BridgeParamKind_File(
      {required final List<String> filter, required this.filterName})
      : _filter = filter,
        super._();

  /// Lower-case extensions without the dot, for the open dialog.
  final List<String> _filter;

  /// Lower-case extensions without the dot, for the open dialog.
  List<String> get filter {
    if (_filter is EqualUnmodifiableListView) return _filter;
    // ignore: implicit_dynamic_type
    return EqualUnmodifiableListView(_filter);
  }

  final String filterName;

  /// Create a copy of BridgeParamKind
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeParamKind_FileCopyWith<BridgeParamKind_File> get copyWith =>
      _$BridgeParamKind_FileCopyWithImpl<BridgeParamKind_File>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeParamKind_File &&
            const DeepCollectionEquality().equals(other._filter, _filter) &&
            (identical(other.filterName, filterName) ||
                other.filterName == filterName));
  }

  @override
  int get hashCode => Object.hash(
      runtimeType, const DeepCollectionEquality().hash(_filter), filterName);

  @override
  String toString() {
    return 'BridgeParamKind.file(filter: $filter, filterName: $filterName)';
  }
}

/// @nodoc
abstract mixin class $BridgeParamKind_FileCopyWith<$Res>
    implements $BridgeParamKindCopyWith<$Res> {
  factory $BridgeParamKind_FileCopyWith(BridgeParamKind_File value,
          $Res Function(BridgeParamKind_File) _then) =
      _$BridgeParamKind_FileCopyWithImpl;
  @useResult
  $Res call({List<String> filter, String filterName});
}

/// @nodoc
class _$BridgeParamKind_FileCopyWithImpl<$Res>
    implements $BridgeParamKind_FileCopyWith<$Res> {
  _$BridgeParamKind_FileCopyWithImpl(this._self, this._then);

  final BridgeParamKind_File _self;
  final $Res Function(BridgeParamKind_File) _then;

  /// Create a copy of BridgeParamKind
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? filter = null,
    Object? filterName = null,
  }) {
    return _then(BridgeParamKind_File(
      filter: null == filter
          ? _self._filter
          : filter // ignore: cast_nullable_to_non_nullable
              as List<String>,
      filterName: null == filterName
          ? _self.filterName
          : filterName // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeParamKind_Layer extends BridgeParamKind {
  const BridgeParamKind_Layer() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeParamKind_Layer);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeParamKind.layer()';
  }
}

/// @nodoc

class BridgeParamKind_MaskPath extends BridgeParamKind {
  const BridgeParamKind_MaskPath() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeParamKind_MaskPath);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeParamKind.maskPath()';
  }
}

/// @nodoc
mixin _$BridgeScalar {
  Object get field0;

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeScalar &&
            const DeepCollectionEquality().equals(other.field0, field0));
  }

  @override
  int get hashCode =>
      Object.hash(runtimeType, const DeepCollectionEquality().hash(field0));

  @override
  String toString() {
    return 'BridgeScalar(field0: $field0)';
  }
}

/// @nodoc
class $BridgeScalarCopyWith<$Res> {
  $BridgeScalarCopyWith(BridgeScalar _, $Res Function(BridgeScalar) __);
}

/// Adds pattern-matching-related methods to [BridgeScalar].
extension BridgeScalarPatterns on BridgeScalar {
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
    TResult Function(BridgeScalar_Static value)? static_,
    TResult Function(BridgeScalar_Keyframed value)? keyframed,
    TResult Function(BridgeScalar_Expression value)? expression,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeScalar_Static() when static_ != null:
        return static_(_that);
      case BridgeScalar_Keyframed() when keyframed != null:
        return keyframed(_that);
      case BridgeScalar_Expression() when expression != null:
        return expression(_that);
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
    required TResult Function(BridgeScalar_Static value) static_,
    required TResult Function(BridgeScalar_Keyframed value) keyframed,
    required TResult Function(BridgeScalar_Expression value) expression,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeScalar_Static():
        return static_(_that);
      case BridgeScalar_Keyframed():
        return keyframed(_that);
      case BridgeScalar_Expression():
        return expression(_that);
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
    TResult? Function(BridgeScalar_Static value)? static_,
    TResult? Function(BridgeScalar_Keyframed value)? keyframed,
    TResult? Function(BridgeScalar_Expression value)? expression,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeScalar_Static() when static_ != null:
        return static_(_that);
      case BridgeScalar_Keyframed() when keyframed != null:
        return keyframed(_that);
      case BridgeScalar_Expression() when expression != null:
        return expression(_that);
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
    TResult Function(double field0)? static_,
    TResult Function(List<BridgeKeyframe> field0)? keyframed,
    TResult Function(String field0)? expression,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeScalar_Static() when static_ != null:
        return static_(_that.field0);
      case BridgeScalar_Keyframed() when keyframed != null:
        return keyframed(_that.field0);
      case BridgeScalar_Expression() when expression != null:
        return expression(_that.field0);
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
    required TResult Function(double field0) static_,
    required TResult Function(List<BridgeKeyframe> field0) keyframed,
    required TResult Function(String field0) expression,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeScalar_Static():
        return static_(_that.field0);
      case BridgeScalar_Keyframed():
        return keyframed(_that.field0);
      case BridgeScalar_Expression():
        return expression(_that.field0);
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
    TResult? Function(double field0)? static_,
    TResult? Function(List<BridgeKeyframe> field0)? keyframed,
    TResult? Function(String field0)? expression,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeScalar_Static() when static_ != null:
        return static_(_that.field0);
      case BridgeScalar_Keyframed() when keyframed != null:
        return keyframed(_that.field0);
      case BridgeScalar_Expression() when expression != null:
        return expression(_that.field0);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeScalar_Static extends BridgeScalar {
  const BridgeScalar_Static(this.field0) : super._();

  @override
  final double field0;

  /// Create a copy of BridgeScalar
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeScalar_StaticCopyWith<BridgeScalar_Static> get copyWith =>
      _$BridgeScalar_StaticCopyWithImpl<BridgeScalar_Static>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeScalar_Static &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeScalar.static_(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeScalar_StaticCopyWith<$Res>
    implements $BridgeScalarCopyWith<$Res> {
  factory $BridgeScalar_StaticCopyWith(
          BridgeScalar_Static value, $Res Function(BridgeScalar_Static) _then) =
      _$BridgeScalar_StaticCopyWithImpl;
  @useResult
  $Res call({double field0});
}

/// @nodoc
class _$BridgeScalar_StaticCopyWithImpl<$Res>
    implements $BridgeScalar_StaticCopyWith<$Res> {
  _$BridgeScalar_StaticCopyWithImpl(this._self, this._then);

  final BridgeScalar_Static _self;
  final $Res Function(BridgeScalar_Static) _then;

  /// Create a copy of BridgeScalar
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeScalar_Static(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as double,
    ));
  }
}

/// @nodoc

class BridgeScalar_Keyframed extends BridgeScalar {
  const BridgeScalar_Keyframed(final List<BridgeKeyframe> field0)
      : _field0 = field0,
        super._();

  final List<BridgeKeyframe> _field0;
  @override
  List<BridgeKeyframe> get field0 {
    if (_field0 is EqualUnmodifiableListView) return _field0;
    // ignore: implicit_dynamic_type
    return EqualUnmodifiableListView(_field0);
  }

  /// Create a copy of BridgeScalar
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeScalar_KeyframedCopyWith<BridgeScalar_Keyframed> get copyWith =>
      _$BridgeScalar_KeyframedCopyWithImpl<BridgeScalar_Keyframed>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeScalar_Keyframed &&
            const DeepCollectionEquality().equals(other._field0, _field0));
  }

  @override
  int get hashCode =>
      Object.hash(runtimeType, const DeepCollectionEquality().hash(_field0));

  @override
  String toString() {
    return 'BridgeScalar.keyframed(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeScalar_KeyframedCopyWith<$Res>
    implements $BridgeScalarCopyWith<$Res> {
  factory $BridgeScalar_KeyframedCopyWith(BridgeScalar_Keyframed value,
          $Res Function(BridgeScalar_Keyframed) _then) =
      _$BridgeScalar_KeyframedCopyWithImpl;
  @useResult
  $Res call({List<BridgeKeyframe> field0});
}

/// @nodoc
class _$BridgeScalar_KeyframedCopyWithImpl<$Res>
    implements $BridgeScalar_KeyframedCopyWith<$Res> {
  _$BridgeScalar_KeyframedCopyWithImpl(this._self, this._then);

  final BridgeScalar_Keyframed _self;
  final $Res Function(BridgeScalar_Keyframed) _then;

  /// Create a copy of BridgeScalar
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeScalar_Keyframed(
      null == field0
          ? _self._field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as List<BridgeKeyframe>,
    ));
  }
}

/// @nodoc

class BridgeScalar_Expression extends BridgeScalar {
  const BridgeScalar_Expression(this.field0) : super._();

  @override
  final String field0;

  /// Create a copy of BridgeScalar
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeScalar_ExpressionCopyWith<BridgeScalar_Expression> get copyWith =>
      _$BridgeScalar_ExpressionCopyWithImpl<BridgeScalar_Expression>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeScalar_Expression &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeScalar.expression(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeScalar_ExpressionCopyWith<$Res>
    implements $BridgeScalarCopyWith<$Res> {
  factory $BridgeScalar_ExpressionCopyWith(BridgeScalar_Expression value,
          $Res Function(BridgeScalar_Expression) _then) =
      _$BridgeScalar_ExpressionCopyWithImpl;
  @useResult
  $Res call({String field0});
}

/// @nodoc
class _$BridgeScalar_ExpressionCopyWithImpl<$Res>
    implements $BridgeScalar_ExpressionCopyWith<$Res> {
  _$BridgeScalar_ExpressionCopyWithImpl(this._self, this._then);

  final BridgeScalar_Expression _self;
  final $Res Function(BridgeScalar_Expression) _then;

  /// Create a copy of BridgeScalar
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeScalar_Expression(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc
mixin _$BridgeSideInterp {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeSideInterp);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeSideInterp()';
  }
}

/// @nodoc
class $BridgeSideInterpCopyWith<$Res> {
  $BridgeSideInterpCopyWith(
      BridgeSideInterp _, $Res Function(BridgeSideInterp) __);
}

/// Adds pattern-matching-related methods to [BridgeSideInterp].
extension BridgeSideInterpPatterns on BridgeSideInterp {
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
    TResult Function(BridgeSideInterp_Hold value)? hold,
    TResult Function(BridgeSideInterp_Linear value)? linear,
    TResult Function(BridgeSideInterp_Bezier value)? bezier,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSideInterp_Hold() when hold != null:
        return hold(_that);
      case BridgeSideInterp_Linear() when linear != null:
        return linear(_that);
      case BridgeSideInterp_Bezier() when bezier != null:
        return bezier(_that);
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
    required TResult Function(BridgeSideInterp_Hold value) hold,
    required TResult Function(BridgeSideInterp_Linear value) linear,
    required TResult Function(BridgeSideInterp_Bezier value) bezier,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSideInterp_Hold():
        return hold(_that);
      case BridgeSideInterp_Linear():
        return linear(_that);
      case BridgeSideInterp_Bezier():
        return bezier(_that);
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
    TResult? Function(BridgeSideInterp_Hold value)? hold,
    TResult? Function(BridgeSideInterp_Linear value)? linear,
    TResult? Function(BridgeSideInterp_Bezier value)? bezier,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSideInterp_Hold() when hold != null:
        return hold(_that);
      case BridgeSideInterp_Linear() when linear != null:
        return linear(_that);
      case BridgeSideInterp_Bezier() when bezier != null:
        return bezier(_that);
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
    TResult Function()? hold,
    TResult Function()? linear,
    TResult Function(BridgeBezierSide field0)? bezier,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSideInterp_Hold() when hold != null:
        return hold();
      case BridgeSideInterp_Linear() when linear != null:
        return linear();
      case BridgeSideInterp_Bezier() when bezier != null:
        return bezier(_that.field0);
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
    required TResult Function() hold,
    required TResult Function() linear,
    required TResult Function(BridgeBezierSide field0) bezier,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSideInterp_Hold():
        return hold();
      case BridgeSideInterp_Linear():
        return linear();
      case BridgeSideInterp_Bezier():
        return bezier(_that.field0);
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
    TResult? Function()? hold,
    TResult? Function()? linear,
    TResult? Function(BridgeBezierSide field0)? bezier,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSideInterp_Hold() when hold != null:
        return hold();
      case BridgeSideInterp_Linear() when linear != null:
        return linear();
      case BridgeSideInterp_Bezier() when bezier != null:
        return bezier(_that.field0);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeSideInterp_Hold extends BridgeSideInterp {
  const BridgeSideInterp_Hold() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeSideInterp_Hold);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeSideInterp.hold()';
  }
}

/// @nodoc

class BridgeSideInterp_Linear extends BridgeSideInterp {
  const BridgeSideInterp_Linear() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeSideInterp_Linear);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeSideInterp.linear()';
  }
}

/// @nodoc

class BridgeSideInterp_Bezier extends BridgeSideInterp {
  const BridgeSideInterp_Bezier(this.field0) : super._();

  final BridgeBezierSide field0;

  /// Create a copy of BridgeSideInterp
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeSideInterp_BezierCopyWith<BridgeSideInterp_Bezier> get copyWith =>
      _$BridgeSideInterp_BezierCopyWithImpl<BridgeSideInterp_Bezier>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeSideInterp_Bezier &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeSideInterp.bezier(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeSideInterp_BezierCopyWith<$Res>
    implements $BridgeSideInterpCopyWith<$Res> {
  factory $BridgeSideInterp_BezierCopyWith(BridgeSideInterp_Bezier value,
          $Res Function(BridgeSideInterp_Bezier) _then) =
      _$BridgeSideInterp_BezierCopyWithImpl;
  @useResult
  $Res call({BridgeBezierSide field0});
}

/// @nodoc
class _$BridgeSideInterp_BezierCopyWithImpl<$Res>
    implements $BridgeSideInterp_BezierCopyWith<$Res> {
  _$BridgeSideInterp_BezierCopyWithImpl(this._self, this._then);

  final BridgeSideInterp_Bezier _self;
  final $Res Function(BridgeSideInterp_Bezier) _then;

  /// Create a copy of BridgeSideInterp
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeSideInterp_Bezier(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeBezierSide,
    ));
  }
}

// dart format on
