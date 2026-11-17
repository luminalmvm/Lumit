// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'template.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$BridgeControlValue {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeControlValue);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeControlValue()';
  }
}

/// @nodoc
class $BridgeControlValueCopyWith<$Res> {
  $BridgeControlValueCopyWith(
      BridgeControlValue _, $Res Function(BridgeControlValue) __);
}

/// Adds pattern-matching-related methods to [BridgeControlValue].
extension BridgeControlValuePatterns on BridgeControlValue {
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
    TResult Function(BridgeControlValue_Number value)? number,
    TResult Function(BridgeControlValue_Point value)? point,
    TResult Function(BridgeControlValue_Colour value)? colour,
    TResult Function(BridgeControlValue_Tick value)? tick,
    TResult Function(BridgeControlValue_Choice value)? choice,
    TResult Function(BridgeControlValue_Text value)? text,
    TResult Function(BridgeControlValue_Font value)? font,
    TResult Function(BridgeControlValue_Picture value)? picture,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeControlValue_Number() when number != null:
        return number(_that);
      case BridgeControlValue_Point() when point != null:
        return point(_that);
      case BridgeControlValue_Colour() when colour != null:
        return colour(_that);
      case BridgeControlValue_Tick() when tick != null:
        return tick(_that);
      case BridgeControlValue_Choice() when choice != null:
        return choice(_that);
      case BridgeControlValue_Text() when text != null:
        return text(_that);
      case BridgeControlValue_Font() when font != null:
        return font(_that);
      case BridgeControlValue_Picture() when picture != null:
        return picture(_that);
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
    required TResult Function(BridgeControlValue_Number value) number,
    required TResult Function(BridgeControlValue_Point value) point,
    required TResult Function(BridgeControlValue_Colour value) colour,
    required TResult Function(BridgeControlValue_Tick value) tick,
    required TResult Function(BridgeControlValue_Choice value) choice,
    required TResult Function(BridgeControlValue_Text value) text,
    required TResult Function(BridgeControlValue_Font value) font,
    required TResult Function(BridgeControlValue_Picture value) picture,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeControlValue_Number():
        return number(_that);
      case BridgeControlValue_Point():
        return point(_that);
      case BridgeControlValue_Colour():
        return colour(_that);
      case BridgeControlValue_Tick():
        return tick(_that);
      case BridgeControlValue_Choice():
        return choice(_that);
      case BridgeControlValue_Text():
        return text(_that);
      case BridgeControlValue_Font():
        return font(_that);
      case BridgeControlValue_Picture():
        return picture(_that);
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
    TResult? Function(BridgeControlValue_Number value)? number,
    TResult? Function(BridgeControlValue_Point value)? point,
    TResult? Function(BridgeControlValue_Colour value)? colour,
    TResult? Function(BridgeControlValue_Tick value)? tick,
    TResult? Function(BridgeControlValue_Choice value)? choice,
    TResult? Function(BridgeControlValue_Text value)? text,
    TResult? Function(BridgeControlValue_Font value)? font,
    TResult? Function(BridgeControlValue_Picture value)? picture,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeControlValue_Number() when number != null:
        return number(_that);
      case BridgeControlValue_Point() when point != null:
        return point(_that);
      case BridgeControlValue_Colour() when colour != null:
        return colour(_that);
      case BridgeControlValue_Tick() when tick != null:
        return tick(_that);
      case BridgeControlValue_Choice() when choice != null:
        return choice(_that);
      case BridgeControlValue_Text() when text != null:
        return text(_that);
      case BridgeControlValue_Font() when font != null:
        return font(_that);
      case BridgeControlValue_Picture() when picture != null:
        return picture(_that);
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
    TResult Function(double field0)? number,
    TResult Function(Float64List field0)? point,
    TResult Function(Float64List field0)? colour,
    TResult Function(bool field0)? tick,
    TResult Function(int field0)? choice,
    TResult Function(String field0)? text,
    TResult Function(String family, String face)? font,
    TResult Function(UuidValue item, int fit)? picture,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeControlValue_Number() when number != null:
        return number(_that.field0);
      case BridgeControlValue_Point() when point != null:
        return point(_that.field0);
      case BridgeControlValue_Colour() when colour != null:
        return colour(_that.field0);
      case BridgeControlValue_Tick() when tick != null:
        return tick(_that.field0);
      case BridgeControlValue_Choice() when choice != null:
        return choice(_that.field0);
      case BridgeControlValue_Text() when text != null:
        return text(_that.field0);
      case BridgeControlValue_Font() when font != null:
        return font(_that.family, _that.face);
      case BridgeControlValue_Picture() when picture != null:
        return picture(_that.item, _that.fit);
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
    required TResult Function(double field0) number,
    required TResult Function(Float64List field0) point,
    required TResult Function(Float64List field0) colour,
    required TResult Function(bool field0) tick,
    required TResult Function(int field0) choice,
    required TResult Function(String field0) text,
    required TResult Function(String family, String face) font,
    required TResult Function(UuidValue item, int fit) picture,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeControlValue_Number():
        return number(_that.field0);
      case BridgeControlValue_Point():
        return point(_that.field0);
      case BridgeControlValue_Colour():
        return colour(_that.field0);
      case BridgeControlValue_Tick():
        return tick(_that.field0);
      case BridgeControlValue_Choice():
        return choice(_that.field0);
      case BridgeControlValue_Text():
        return text(_that.field0);
      case BridgeControlValue_Font():
        return font(_that.family, _that.face);
      case BridgeControlValue_Picture():
        return picture(_that.item, _that.fit);
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
    TResult? Function(double field0)? number,
    TResult? Function(Float64List field0)? point,
    TResult? Function(Float64List field0)? colour,
    TResult? Function(bool field0)? tick,
    TResult? Function(int field0)? choice,
    TResult? Function(String field0)? text,
    TResult? Function(String family, String face)? font,
    TResult? Function(UuidValue item, int fit)? picture,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeControlValue_Number() when number != null:
        return number(_that.field0);
      case BridgeControlValue_Point() when point != null:
        return point(_that.field0);
      case BridgeControlValue_Colour() when colour != null:
        return colour(_that.field0);
      case BridgeControlValue_Tick() when tick != null:
        return tick(_that.field0);
      case BridgeControlValue_Choice() when choice != null:
        return choice(_that.field0);
      case BridgeControlValue_Text() when text != null:
        return text(_that.field0);
      case BridgeControlValue_Font() when font != null:
        return font(_that.family, _that.face);
      case BridgeControlValue_Picture() when picture != null:
        return picture(_that.item, _that.fit);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeControlValue_Number extends BridgeControlValue {
  const BridgeControlValue_Number(this.field0) : super._();

  final double field0;

  /// Create a copy of BridgeControlValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeControlValue_NumberCopyWith<BridgeControlValue_Number> get copyWith =>
      _$BridgeControlValue_NumberCopyWithImpl<BridgeControlValue_Number>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeControlValue_Number &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeControlValue.number(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeControlValue_NumberCopyWith<$Res>
    implements $BridgeControlValueCopyWith<$Res> {
  factory $BridgeControlValue_NumberCopyWith(BridgeControlValue_Number value,
          $Res Function(BridgeControlValue_Number) _then) =
      _$BridgeControlValue_NumberCopyWithImpl;
  @useResult
  $Res call({double field0});
}

/// @nodoc
class _$BridgeControlValue_NumberCopyWithImpl<$Res>
    implements $BridgeControlValue_NumberCopyWith<$Res> {
  _$BridgeControlValue_NumberCopyWithImpl(this._self, this._then);

  final BridgeControlValue_Number _self;
  final $Res Function(BridgeControlValue_Number) _then;

  /// Create a copy of BridgeControlValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeControlValue_Number(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as double,
    ));
  }
}

/// @nodoc

class BridgeControlValue_Point extends BridgeControlValue {
  const BridgeControlValue_Point(this.field0) : super._();

  final Float64List field0;

  /// Create a copy of BridgeControlValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeControlValue_PointCopyWith<BridgeControlValue_Point> get copyWith =>
      _$BridgeControlValue_PointCopyWithImpl<BridgeControlValue_Point>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeControlValue_Point &&
            const DeepCollectionEquality().equals(other.field0, field0));
  }

  @override
  int get hashCode =>
      Object.hash(runtimeType, const DeepCollectionEquality().hash(field0));

  @override
  String toString() {
    return 'BridgeControlValue.point(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeControlValue_PointCopyWith<$Res>
    implements $BridgeControlValueCopyWith<$Res> {
  factory $BridgeControlValue_PointCopyWith(BridgeControlValue_Point value,
          $Res Function(BridgeControlValue_Point) _then) =
      _$BridgeControlValue_PointCopyWithImpl;
  @useResult
  $Res call({Float64List field0});
}

/// @nodoc
class _$BridgeControlValue_PointCopyWithImpl<$Res>
    implements $BridgeControlValue_PointCopyWith<$Res> {
  _$BridgeControlValue_PointCopyWithImpl(this._self, this._then);

  final BridgeControlValue_Point _self;
  final $Res Function(BridgeControlValue_Point) _then;

  /// Create a copy of BridgeControlValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeControlValue_Point(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as Float64List,
    ));
  }
}

/// @nodoc

class BridgeControlValue_Colour extends BridgeControlValue {
  const BridgeControlValue_Colour(this.field0) : super._();

  final Float64List field0;

  /// Create a copy of BridgeControlValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeControlValue_ColourCopyWith<BridgeControlValue_Colour> get copyWith =>
      _$BridgeControlValue_ColourCopyWithImpl<BridgeControlValue_Colour>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeControlValue_Colour &&
            const DeepCollectionEquality().equals(other.field0, field0));
  }

  @override
  int get hashCode =>
      Object.hash(runtimeType, const DeepCollectionEquality().hash(field0));

  @override
  String toString() {
    return 'BridgeControlValue.colour(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeControlValue_ColourCopyWith<$Res>
    implements $BridgeControlValueCopyWith<$Res> {
  factory $BridgeControlValue_ColourCopyWith(BridgeControlValue_Colour value,
          $Res Function(BridgeControlValue_Colour) _then) =
      _$BridgeControlValue_ColourCopyWithImpl;
  @useResult
  $Res call({Float64List field0});
}

/// @nodoc
class _$BridgeControlValue_ColourCopyWithImpl<$Res>
    implements $BridgeControlValue_ColourCopyWith<$Res> {
  _$BridgeControlValue_ColourCopyWithImpl(this._self, this._then);

  final BridgeControlValue_Colour _self;
  final $Res Function(BridgeControlValue_Colour) _then;

  /// Create a copy of BridgeControlValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeControlValue_Colour(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as Float64List,
    ));
  }
}

/// @nodoc

class BridgeControlValue_Tick extends BridgeControlValue {
  const BridgeControlValue_Tick(this.field0) : super._();

  final bool field0;

  /// Create a copy of BridgeControlValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeControlValue_TickCopyWith<BridgeControlValue_Tick> get copyWith =>
      _$BridgeControlValue_TickCopyWithImpl<BridgeControlValue_Tick>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeControlValue_Tick &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeControlValue.tick(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeControlValue_TickCopyWith<$Res>
    implements $BridgeControlValueCopyWith<$Res> {
  factory $BridgeControlValue_TickCopyWith(BridgeControlValue_Tick value,
          $Res Function(BridgeControlValue_Tick) _then) =
      _$BridgeControlValue_TickCopyWithImpl;
  @useResult
  $Res call({bool field0});
}

/// @nodoc
class _$BridgeControlValue_TickCopyWithImpl<$Res>
    implements $BridgeControlValue_TickCopyWith<$Res> {
  _$BridgeControlValue_TickCopyWithImpl(this._self, this._then);

  final BridgeControlValue_Tick _self;
  final $Res Function(BridgeControlValue_Tick) _then;

  /// Create a copy of BridgeControlValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeControlValue_Tick(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as bool,
    ));
  }
}

/// @nodoc

class BridgeControlValue_Choice extends BridgeControlValue {
  const BridgeControlValue_Choice(this.field0) : super._();

  final int field0;

  /// Create a copy of BridgeControlValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeControlValue_ChoiceCopyWith<BridgeControlValue_Choice> get copyWith =>
      _$BridgeControlValue_ChoiceCopyWithImpl<BridgeControlValue_Choice>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeControlValue_Choice &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeControlValue.choice(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeControlValue_ChoiceCopyWith<$Res>
    implements $BridgeControlValueCopyWith<$Res> {
  factory $BridgeControlValue_ChoiceCopyWith(BridgeControlValue_Choice value,
          $Res Function(BridgeControlValue_Choice) _then) =
      _$BridgeControlValue_ChoiceCopyWithImpl;
  @useResult
  $Res call({int field0});
}

/// @nodoc
class _$BridgeControlValue_ChoiceCopyWithImpl<$Res>
    implements $BridgeControlValue_ChoiceCopyWith<$Res> {
  _$BridgeControlValue_ChoiceCopyWithImpl(this._self, this._then);

  final BridgeControlValue_Choice _self;
  final $Res Function(BridgeControlValue_Choice) _then;

  /// Create a copy of BridgeControlValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeControlValue_Choice(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as int,
    ));
  }
}

/// @nodoc

class BridgeControlValue_Text extends BridgeControlValue {
  const BridgeControlValue_Text(this.field0) : super._();

  final String field0;

  /// Create a copy of BridgeControlValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeControlValue_TextCopyWith<BridgeControlValue_Text> get copyWith =>
      _$BridgeControlValue_TextCopyWithImpl<BridgeControlValue_Text>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeControlValue_Text &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeControlValue.text(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeControlValue_TextCopyWith<$Res>
    implements $BridgeControlValueCopyWith<$Res> {
  factory $BridgeControlValue_TextCopyWith(BridgeControlValue_Text value,
          $Res Function(BridgeControlValue_Text) _then) =
      _$BridgeControlValue_TextCopyWithImpl;
  @useResult
  $Res call({String field0});
}

/// @nodoc
class _$BridgeControlValue_TextCopyWithImpl<$Res>
    implements $BridgeControlValue_TextCopyWith<$Res> {
  _$BridgeControlValue_TextCopyWithImpl(this._self, this._then);

  final BridgeControlValue_Text _self;
  final $Res Function(BridgeControlValue_Text) _then;

  /// Create a copy of BridgeControlValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeControlValue_Text(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeControlValue_Font extends BridgeControlValue {
  const BridgeControlValue_Font({required this.family, required this.face})
      : super._();

  final String family;
  final String face;

  /// Create a copy of BridgeControlValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeControlValue_FontCopyWith<BridgeControlValue_Font> get copyWith =>
      _$BridgeControlValue_FontCopyWithImpl<BridgeControlValue_Font>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeControlValue_Font &&
            (identical(other.family, family) || other.family == family) &&
            (identical(other.face, face) || other.face == face));
  }

  @override
  int get hashCode => Object.hash(runtimeType, family, face);

  @override
  String toString() {
    return 'BridgeControlValue.font(family: $family, face: $face)';
  }
}

/// @nodoc
abstract mixin class $BridgeControlValue_FontCopyWith<$Res>
    implements $BridgeControlValueCopyWith<$Res> {
  factory $BridgeControlValue_FontCopyWith(BridgeControlValue_Font value,
          $Res Function(BridgeControlValue_Font) _then) =
      _$BridgeControlValue_FontCopyWithImpl;
  @useResult
  $Res call({String family, String face});
}

/// @nodoc
class _$BridgeControlValue_FontCopyWithImpl<$Res>
    implements $BridgeControlValue_FontCopyWith<$Res> {
  _$BridgeControlValue_FontCopyWithImpl(this._self, this._then);

  final BridgeControlValue_Font _self;
  final $Res Function(BridgeControlValue_Font) _then;

  /// Create a copy of BridgeControlValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? family = null,
    Object? face = null,
  }) {
    return _then(BridgeControlValue_Font(
      family: null == family
          ? _self.family
          : family // ignore: cast_nullable_to_non_nullable
              as String,
      face: null == face
          ? _self.face
          : face // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeControlValue_Picture extends BridgeControlValue {
  const BridgeControlValue_Picture({required this.item, required this.fit})
      : super._();

  final UuidValue item;
  final int fit;

  /// Create a copy of BridgeControlValue
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeControlValue_PictureCopyWith<BridgeControlValue_Picture>
      get copyWith =>
          _$BridgeControlValue_PictureCopyWithImpl<BridgeControlValue_Picture>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeControlValue_Picture &&
            (identical(other.item, item) || other.item == item) &&
            (identical(other.fit, fit) || other.fit == fit));
  }

  @override
  int get hashCode => Object.hash(runtimeType, item, fit);

  @override
  String toString() {
    return 'BridgeControlValue.picture(item: $item, fit: $fit)';
  }
}

/// @nodoc
abstract mixin class $BridgeControlValue_PictureCopyWith<$Res>
    implements $BridgeControlValueCopyWith<$Res> {
  factory $BridgeControlValue_PictureCopyWith(BridgeControlValue_Picture value,
          $Res Function(BridgeControlValue_Picture) _then) =
      _$BridgeControlValue_PictureCopyWithImpl;
  @useResult
  $Res call({UuidValue item, int fit});
}

/// @nodoc
class _$BridgeControlValue_PictureCopyWithImpl<$Res>
    implements $BridgeControlValue_PictureCopyWith<$Res> {
  _$BridgeControlValue_PictureCopyWithImpl(this._self, this._then);

  final BridgeControlValue_Picture _self;
  final $Res Function(BridgeControlValue_Picture) _then;

  /// Create a copy of BridgeControlValue
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? item = null,
    Object? fit = null,
  }) {
    return _then(BridgeControlValue_Picture(
      item: null == item
          ? _self.item
          : item // ignore: cast_nullable_to_non_nullable
              as UuidValue,
      fit: null == fit
          ? _self.fit
          : fit // ignore: cast_nullable_to_non_nullable
              as int,
    ));
  }
}

// dart format on
