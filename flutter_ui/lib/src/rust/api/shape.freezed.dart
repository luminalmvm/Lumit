// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'shape.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$BridgeShapeEntryKind {
  Object get field0;

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShapeEntryKind &&
            const DeepCollectionEquality().equals(other.field0, field0));
  }

  @override
  int get hashCode =>
      Object.hash(runtimeType, const DeepCollectionEquality().hash(field0));

  @override
  String toString() {
    return 'BridgeShapeEntryKind(field0: $field0)';
  }
}

/// @nodoc
class $BridgeShapeEntryKindCopyWith<$Res> {
  $BridgeShapeEntryKindCopyWith(
      BridgeShapeEntryKind _, $Res Function(BridgeShapeEntryKind) __);
}

/// Adds pattern-matching-related methods to [BridgeShapeEntryKind].
extension BridgeShapeEntryKindPatterns on BridgeShapeEntryKind {
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
    TResult Function(BridgeShapeEntryKind_Item value)? item,
    TResult Function(BridgeShapeEntryKind_Modifier value)? modifier,
    TResult Function(BridgeShapeEntryKind_Group value)? group,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShapeEntryKind_Item() when item != null:
        return item(_that);
      case BridgeShapeEntryKind_Modifier() when modifier != null:
        return modifier(_that);
      case BridgeShapeEntryKind_Group() when group != null:
        return group(_that);
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
    required TResult Function(BridgeShapeEntryKind_Item value) item,
    required TResult Function(BridgeShapeEntryKind_Modifier value) modifier,
    required TResult Function(BridgeShapeEntryKind_Group value) group,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShapeEntryKind_Item():
        return item(_that);
      case BridgeShapeEntryKind_Modifier():
        return modifier(_that);
      case BridgeShapeEntryKind_Group():
        return group(_that);
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
    TResult? Function(BridgeShapeEntryKind_Item value)? item,
    TResult? Function(BridgeShapeEntryKind_Modifier value)? modifier,
    TResult? Function(BridgeShapeEntryKind_Group value)? group,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShapeEntryKind_Item() when item != null:
        return item(_that);
      case BridgeShapeEntryKind_Modifier() when modifier != null:
        return modifier(_that);
      case BridgeShapeEntryKind_Group() when group != null:
        return group(_that);
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
    TResult Function(BridgeShapeItem field0)? item,
    TResult Function(BridgeShapeModifier field0)? modifier,
    TResult Function(BridgeShapeGroup field0)? group,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShapeEntryKind_Item() when item != null:
        return item(_that.field0);
      case BridgeShapeEntryKind_Modifier() when modifier != null:
        return modifier(_that.field0);
      case BridgeShapeEntryKind_Group() when group != null:
        return group(_that.field0);
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
    required TResult Function(BridgeShapeItem field0) item,
    required TResult Function(BridgeShapeModifier field0) modifier,
    required TResult Function(BridgeShapeGroup field0) group,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShapeEntryKind_Item():
        return item(_that.field0);
      case BridgeShapeEntryKind_Modifier():
        return modifier(_that.field0);
      case BridgeShapeEntryKind_Group():
        return group(_that.field0);
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
    TResult? Function(BridgeShapeItem field0)? item,
    TResult? Function(BridgeShapeModifier field0)? modifier,
    TResult? Function(BridgeShapeGroup field0)? group,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShapeEntryKind_Item() when item != null:
        return item(_that.field0);
      case BridgeShapeEntryKind_Modifier() when modifier != null:
        return modifier(_that.field0);
      case BridgeShapeEntryKind_Group() when group != null:
        return group(_that.field0);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeShapeEntryKind_Item extends BridgeShapeEntryKind {
  const BridgeShapeEntryKind_Item(this.field0) : super._();

  @override
  final BridgeShapeItem field0;

  /// Create a copy of BridgeShapeEntryKind
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShapeEntryKind_ItemCopyWith<BridgeShapeEntryKind_Item> get copyWith =>
      _$BridgeShapeEntryKind_ItemCopyWithImpl<BridgeShapeEntryKind_Item>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShapeEntryKind_Item &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeShapeEntryKind.item(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeShapeEntryKind_ItemCopyWith<$Res>
    implements $BridgeShapeEntryKindCopyWith<$Res> {
  factory $BridgeShapeEntryKind_ItemCopyWith(BridgeShapeEntryKind_Item value,
          $Res Function(BridgeShapeEntryKind_Item) _then) =
      _$BridgeShapeEntryKind_ItemCopyWithImpl;
  @useResult
  $Res call({BridgeShapeItem field0});
}

/// @nodoc
class _$BridgeShapeEntryKind_ItemCopyWithImpl<$Res>
    implements $BridgeShapeEntryKind_ItemCopyWith<$Res> {
  _$BridgeShapeEntryKind_ItemCopyWithImpl(this._self, this._then);

  final BridgeShapeEntryKind_Item _self;
  final $Res Function(BridgeShapeEntryKind_Item) _then;

  /// Create a copy of BridgeShapeEntryKind
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeShapeEntryKind_Item(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeShapeItem,
    ));
  }
}

/// @nodoc

class BridgeShapeEntryKind_Modifier extends BridgeShapeEntryKind {
  const BridgeShapeEntryKind_Modifier(this.field0) : super._();

  @override
  final BridgeShapeModifier field0;

  /// Create a copy of BridgeShapeEntryKind
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShapeEntryKind_ModifierCopyWith<BridgeShapeEntryKind_Modifier>
      get copyWith => _$BridgeShapeEntryKind_ModifierCopyWithImpl<
          BridgeShapeEntryKind_Modifier>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShapeEntryKind_Modifier &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeShapeEntryKind.modifier(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeShapeEntryKind_ModifierCopyWith<$Res>
    implements $BridgeShapeEntryKindCopyWith<$Res> {
  factory $BridgeShapeEntryKind_ModifierCopyWith(
          BridgeShapeEntryKind_Modifier value,
          $Res Function(BridgeShapeEntryKind_Modifier) _then) =
      _$BridgeShapeEntryKind_ModifierCopyWithImpl;
  @useResult
  $Res call({BridgeShapeModifier field0});
}

/// @nodoc
class _$BridgeShapeEntryKind_ModifierCopyWithImpl<$Res>
    implements $BridgeShapeEntryKind_ModifierCopyWith<$Res> {
  _$BridgeShapeEntryKind_ModifierCopyWithImpl(this._self, this._then);

  final BridgeShapeEntryKind_Modifier _self;
  final $Res Function(BridgeShapeEntryKind_Modifier) _then;

  /// Create a copy of BridgeShapeEntryKind
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeShapeEntryKind_Modifier(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeShapeModifier,
    ));
  }
}

/// @nodoc

class BridgeShapeEntryKind_Group extends BridgeShapeEntryKind {
  const BridgeShapeEntryKind_Group(this.field0) : super._();

  @override
  final BridgeShapeGroup field0;

  /// Create a copy of BridgeShapeEntryKind
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShapeEntryKind_GroupCopyWith<BridgeShapeEntryKind_Group>
      get copyWith =>
          _$BridgeShapeEntryKind_GroupCopyWithImpl<BridgeShapeEntryKind_Group>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShapeEntryKind_Group &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeShapeEntryKind.group(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeShapeEntryKind_GroupCopyWith<$Res>
    implements $BridgeShapeEntryKindCopyWith<$Res> {
  factory $BridgeShapeEntryKind_GroupCopyWith(BridgeShapeEntryKind_Group value,
          $Res Function(BridgeShapeEntryKind_Group) _then) =
      _$BridgeShapeEntryKind_GroupCopyWithImpl;
  @useResult
  $Res call({BridgeShapeGroup field0});
}

/// @nodoc
class _$BridgeShapeEntryKind_GroupCopyWithImpl<$Res>
    implements $BridgeShapeEntryKind_GroupCopyWith<$Res> {
  _$BridgeShapeEntryKind_GroupCopyWithImpl(this._self, this._then);

  final BridgeShapeEntryKind_Group _self;
  final $Res Function(BridgeShapeEntryKind_Group) _then;

  /// Create a copy of BridgeShapeEntryKind
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeShapeEntryKind_Group(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeShapeGroup,
    ));
  }
}

/// @nodoc
mixin _$BridgeShapeModifierKind {
  Object get field0;

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShapeModifierKind &&
            const DeepCollectionEquality().equals(other.field0, field0));
  }

  @override
  int get hashCode =>
      Object.hash(runtimeType, const DeepCollectionEquality().hash(field0));

  @override
  String toString() {
    return 'BridgeShapeModifierKind(field0: $field0)';
  }
}

/// @nodoc
class $BridgeShapeModifierKindCopyWith<$Res> {
  $BridgeShapeModifierKindCopyWith(
      BridgeShapeModifierKind _, $Res Function(BridgeShapeModifierKind) __);
}

/// Adds pattern-matching-related methods to [BridgeShapeModifierKind].
extension BridgeShapeModifierKindPatterns on BridgeShapeModifierKind {
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
    TResult Function(BridgeShapeModifierKind_Repeat value)? repeat,
    TResult Function(BridgeShapeModifierKind_Trim value)? trim,
    TResult Function(BridgeShapeModifierKind_Offset value)? offset,
    TResult Function(BridgeShapeModifierKind_Twist value)? twist,
    TResult Function(BridgeShapeModifierKind_Deform value)? deform,
    TResult Function(BridgeShapeModifierKind_Smooth value)? smooth,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShapeModifierKind_Repeat() when repeat != null:
        return repeat(_that);
      case BridgeShapeModifierKind_Trim() when trim != null:
        return trim(_that);
      case BridgeShapeModifierKind_Offset() when offset != null:
        return offset(_that);
      case BridgeShapeModifierKind_Twist() when twist != null:
        return twist(_that);
      case BridgeShapeModifierKind_Deform() when deform != null:
        return deform(_that);
      case BridgeShapeModifierKind_Smooth() when smooth != null:
        return smooth(_that);
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
    required TResult Function(BridgeShapeModifierKind_Repeat value) repeat,
    required TResult Function(BridgeShapeModifierKind_Trim value) trim,
    required TResult Function(BridgeShapeModifierKind_Offset value) offset,
    required TResult Function(BridgeShapeModifierKind_Twist value) twist,
    required TResult Function(BridgeShapeModifierKind_Deform value) deform,
    required TResult Function(BridgeShapeModifierKind_Smooth value) smooth,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShapeModifierKind_Repeat():
        return repeat(_that);
      case BridgeShapeModifierKind_Trim():
        return trim(_that);
      case BridgeShapeModifierKind_Offset():
        return offset(_that);
      case BridgeShapeModifierKind_Twist():
        return twist(_that);
      case BridgeShapeModifierKind_Deform():
        return deform(_that);
      case BridgeShapeModifierKind_Smooth():
        return smooth(_that);
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
    TResult? Function(BridgeShapeModifierKind_Repeat value)? repeat,
    TResult? Function(BridgeShapeModifierKind_Trim value)? trim,
    TResult? Function(BridgeShapeModifierKind_Offset value)? offset,
    TResult? Function(BridgeShapeModifierKind_Twist value)? twist,
    TResult? Function(BridgeShapeModifierKind_Deform value)? deform,
    TResult? Function(BridgeShapeModifierKind_Smooth value)? smooth,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShapeModifierKind_Repeat() when repeat != null:
        return repeat(_that);
      case BridgeShapeModifierKind_Trim() when trim != null:
        return trim(_that);
      case BridgeShapeModifierKind_Offset() when offset != null:
        return offset(_that);
      case BridgeShapeModifierKind_Twist() when twist != null:
        return twist(_that);
      case BridgeShapeModifierKind_Deform() when deform != null:
        return deform(_that);
      case BridgeShapeModifierKind_Smooth() when smooth != null:
        return smooth(_that);
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
    TResult Function(BridgeShapeRepeater field0)? repeat,
    TResult Function(BridgeShapeTrim field0)? trim,
    TResult Function(BridgeScalar field0)? offset,
    TResult Function(BridgeShapeTwist field0)? twist,
    TResult Function(BridgeShapeDeform field0)? deform,
    TResult Function(BridgeScalar field0)? smooth,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShapeModifierKind_Repeat() when repeat != null:
        return repeat(_that.field0);
      case BridgeShapeModifierKind_Trim() when trim != null:
        return trim(_that.field0);
      case BridgeShapeModifierKind_Offset() when offset != null:
        return offset(_that.field0);
      case BridgeShapeModifierKind_Twist() when twist != null:
        return twist(_that.field0);
      case BridgeShapeModifierKind_Deform() when deform != null:
        return deform(_that.field0);
      case BridgeShapeModifierKind_Smooth() when smooth != null:
        return smooth(_that.field0);
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
    required TResult Function(BridgeShapeRepeater field0) repeat,
    required TResult Function(BridgeShapeTrim field0) trim,
    required TResult Function(BridgeScalar field0) offset,
    required TResult Function(BridgeShapeTwist field0) twist,
    required TResult Function(BridgeShapeDeform field0) deform,
    required TResult Function(BridgeScalar field0) smooth,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShapeModifierKind_Repeat():
        return repeat(_that.field0);
      case BridgeShapeModifierKind_Trim():
        return trim(_that.field0);
      case BridgeShapeModifierKind_Offset():
        return offset(_that.field0);
      case BridgeShapeModifierKind_Twist():
        return twist(_that.field0);
      case BridgeShapeModifierKind_Deform():
        return deform(_that.field0);
      case BridgeShapeModifierKind_Smooth():
        return smooth(_that.field0);
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
    TResult? Function(BridgeShapeRepeater field0)? repeat,
    TResult? Function(BridgeShapeTrim field0)? trim,
    TResult? Function(BridgeScalar field0)? offset,
    TResult? Function(BridgeShapeTwist field0)? twist,
    TResult? Function(BridgeShapeDeform field0)? deform,
    TResult? Function(BridgeScalar field0)? smooth,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShapeModifierKind_Repeat() when repeat != null:
        return repeat(_that.field0);
      case BridgeShapeModifierKind_Trim() when trim != null:
        return trim(_that.field0);
      case BridgeShapeModifierKind_Offset() when offset != null:
        return offset(_that.field0);
      case BridgeShapeModifierKind_Twist() when twist != null:
        return twist(_that.field0);
      case BridgeShapeModifierKind_Deform() when deform != null:
        return deform(_that.field0);
      case BridgeShapeModifierKind_Smooth() when smooth != null:
        return smooth(_that.field0);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeShapeModifierKind_Repeat extends BridgeShapeModifierKind {
  const BridgeShapeModifierKind_Repeat(this.field0) : super._();

  @override
  final BridgeShapeRepeater field0;

  /// Create a copy of BridgeShapeModifierKind
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShapeModifierKind_RepeatCopyWith<BridgeShapeModifierKind_Repeat>
      get copyWith => _$BridgeShapeModifierKind_RepeatCopyWithImpl<
          BridgeShapeModifierKind_Repeat>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShapeModifierKind_Repeat &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeShapeModifierKind.repeat(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeShapeModifierKind_RepeatCopyWith<$Res>
    implements $BridgeShapeModifierKindCopyWith<$Res> {
  factory $BridgeShapeModifierKind_RepeatCopyWith(
          BridgeShapeModifierKind_Repeat value,
          $Res Function(BridgeShapeModifierKind_Repeat) _then) =
      _$BridgeShapeModifierKind_RepeatCopyWithImpl;
  @useResult
  $Res call({BridgeShapeRepeater field0});
}

/// @nodoc
class _$BridgeShapeModifierKind_RepeatCopyWithImpl<$Res>
    implements $BridgeShapeModifierKind_RepeatCopyWith<$Res> {
  _$BridgeShapeModifierKind_RepeatCopyWithImpl(this._self, this._then);

  final BridgeShapeModifierKind_Repeat _self;
  final $Res Function(BridgeShapeModifierKind_Repeat) _then;

  /// Create a copy of BridgeShapeModifierKind
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeShapeModifierKind_Repeat(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeShapeRepeater,
    ));
  }
}

/// @nodoc

class BridgeShapeModifierKind_Trim extends BridgeShapeModifierKind {
  const BridgeShapeModifierKind_Trim(this.field0) : super._();

  @override
  final BridgeShapeTrim field0;

  /// Create a copy of BridgeShapeModifierKind
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShapeModifierKind_TrimCopyWith<BridgeShapeModifierKind_Trim>
      get copyWith => _$BridgeShapeModifierKind_TrimCopyWithImpl<
          BridgeShapeModifierKind_Trim>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShapeModifierKind_Trim &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeShapeModifierKind.trim(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeShapeModifierKind_TrimCopyWith<$Res>
    implements $BridgeShapeModifierKindCopyWith<$Res> {
  factory $BridgeShapeModifierKind_TrimCopyWith(
          BridgeShapeModifierKind_Trim value,
          $Res Function(BridgeShapeModifierKind_Trim) _then) =
      _$BridgeShapeModifierKind_TrimCopyWithImpl;
  @useResult
  $Res call({BridgeShapeTrim field0});
}

/// @nodoc
class _$BridgeShapeModifierKind_TrimCopyWithImpl<$Res>
    implements $BridgeShapeModifierKind_TrimCopyWith<$Res> {
  _$BridgeShapeModifierKind_TrimCopyWithImpl(this._self, this._then);

  final BridgeShapeModifierKind_Trim _self;
  final $Res Function(BridgeShapeModifierKind_Trim) _then;

  /// Create a copy of BridgeShapeModifierKind
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeShapeModifierKind_Trim(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeShapeTrim,
    ));
  }
}

/// @nodoc

class BridgeShapeModifierKind_Offset extends BridgeShapeModifierKind {
  const BridgeShapeModifierKind_Offset(this.field0) : super._();

  @override
  final BridgeScalar field0;

  /// Create a copy of BridgeShapeModifierKind
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShapeModifierKind_OffsetCopyWith<BridgeShapeModifierKind_Offset>
      get copyWith => _$BridgeShapeModifierKind_OffsetCopyWithImpl<
          BridgeShapeModifierKind_Offset>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShapeModifierKind_Offset &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeShapeModifierKind.offset(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeShapeModifierKind_OffsetCopyWith<$Res>
    implements $BridgeShapeModifierKindCopyWith<$Res> {
  factory $BridgeShapeModifierKind_OffsetCopyWith(
          BridgeShapeModifierKind_Offset value,
          $Res Function(BridgeShapeModifierKind_Offset) _then) =
      _$BridgeShapeModifierKind_OffsetCopyWithImpl;
  @useResult
  $Res call({BridgeScalar field0});

  $BridgeScalarCopyWith<$Res> get field0;
}

/// @nodoc
class _$BridgeShapeModifierKind_OffsetCopyWithImpl<$Res>
    implements $BridgeShapeModifierKind_OffsetCopyWith<$Res> {
  _$BridgeShapeModifierKind_OffsetCopyWithImpl(this._self, this._then);

  final BridgeShapeModifierKind_Offset _self;
  final $Res Function(BridgeShapeModifierKind_Offset) _then;

  /// Create a copy of BridgeShapeModifierKind
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeShapeModifierKind_Offset(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeScalar,
    ));
  }

  /// Create a copy of BridgeShapeModifierKind
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

class BridgeShapeModifierKind_Twist extends BridgeShapeModifierKind {
  const BridgeShapeModifierKind_Twist(this.field0) : super._();

  @override
  final BridgeShapeTwist field0;

  /// Create a copy of BridgeShapeModifierKind
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShapeModifierKind_TwistCopyWith<BridgeShapeModifierKind_Twist>
      get copyWith => _$BridgeShapeModifierKind_TwistCopyWithImpl<
          BridgeShapeModifierKind_Twist>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShapeModifierKind_Twist &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeShapeModifierKind.twist(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeShapeModifierKind_TwistCopyWith<$Res>
    implements $BridgeShapeModifierKindCopyWith<$Res> {
  factory $BridgeShapeModifierKind_TwistCopyWith(
          BridgeShapeModifierKind_Twist value,
          $Res Function(BridgeShapeModifierKind_Twist) _then) =
      _$BridgeShapeModifierKind_TwistCopyWithImpl;
  @useResult
  $Res call({BridgeShapeTwist field0});
}

/// @nodoc
class _$BridgeShapeModifierKind_TwistCopyWithImpl<$Res>
    implements $BridgeShapeModifierKind_TwistCopyWith<$Res> {
  _$BridgeShapeModifierKind_TwistCopyWithImpl(this._self, this._then);

  final BridgeShapeModifierKind_Twist _self;
  final $Res Function(BridgeShapeModifierKind_Twist) _then;

  /// Create a copy of BridgeShapeModifierKind
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeShapeModifierKind_Twist(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeShapeTwist,
    ));
  }
}

/// @nodoc

class BridgeShapeModifierKind_Deform extends BridgeShapeModifierKind {
  const BridgeShapeModifierKind_Deform(this.field0) : super._();

  @override
  final BridgeShapeDeform field0;

  /// Create a copy of BridgeShapeModifierKind
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShapeModifierKind_DeformCopyWith<BridgeShapeModifierKind_Deform>
      get copyWith => _$BridgeShapeModifierKind_DeformCopyWithImpl<
          BridgeShapeModifierKind_Deform>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShapeModifierKind_Deform &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeShapeModifierKind.deform(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeShapeModifierKind_DeformCopyWith<$Res>
    implements $BridgeShapeModifierKindCopyWith<$Res> {
  factory $BridgeShapeModifierKind_DeformCopyWith(
          BridgeShapeModifierKind_Deform value,
          $Res Function(BridgeShapeModifierKind_Deform) _then) =
      _$BridgeShapeModifierKind_DeformCopyWithImpl;
  @useResult
  $Res call({BridgeShapeDeform field0});
}

/// @nodoc
class _$BridgeShapeModifierKind_DeformCopyWithImpl<$Res>
    implements $BridgeShapeModifierKind_DeformCopyWith<$Res> {
  _$BridgeShapeModifierKind_DeformCopyWithImpl(this._self, this._then);

  final BridgeShapeModifierKind_Deform _self;
  final $Res Function(BridgeShapeModifierKind_Deform) _then;

  /// Create a copy of BridgeShapeModifierKind
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeShapeModifierKind_Deform(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeShapeDeform,
    ));
  }
}

/// @nodoc

class BridgeShapeModifierKind_Smooth extends BridgeShapeModifierKind {
  const BridgeShapeModifierKind_Smooth(this.field0) : super._();

  @override
  final BridgeScalar field0;

  /// Create a copy of BridgeShapeModifierKind
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShapeModifierKind_SmoothCopyWith<BridgeShapeModifierKind_Smooth>
      get copyWith => _$BridgeShapeModifierKind_SmoothCopyWithImpl<
          BridgeShapeModifierKind_Smooth>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShapeModifierKind_Smooth &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeShapeModifierKind.smooth(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeShapeModifierKind_SmoothCopyWith<$Res>
    implements $BridgeShapeModifierKindCopyWith<$Res> {
  factory $BridgeShapeModifierKind_SmoothCopyWith(
          BridgeShapeModifierKind_Smooth value,
          $Res Function(BridgeShapeModifierKind_Smooth) _then) =
      _$BridgeShapeModifierKind_SmoothCopyWithImpl;
  @useResult
  $Res call({BridgeScalar field0});

  $BridgeScalarCopyWith<$Res> get field0;
}

/// @nodoc
class _$BridgeShapeModifierKind_SmoothCopyWithImpl<$Res>
    implements $BridgeShapeModifierKind_SmoothCopyWith<$Res> {
  _$BridgeShapeModifierKind_SmoothCopyWithImpl(this._self, this._then);

  final BridgeShapeModifierKind_Smooth _self;
  final $Res Function(BridgeShapeModifierKind_Smooth) _then;

  /// Create a copy of BridgeShapeModifierKind
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeShapeModifierKind_Smooth(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeScalar,
    ));
  }

  /// Create a copy of BridgeShapeModifierKind
  /// with the given fields replaced by the non-null parameter values.
  @override
  @pragma('vm:prefer-inline')
  $BridgeScalarCopyWith<$Res> get field0 {
    return $BridgeScalarCopyWith<$Res>(_self.field0, (value) {
      return _then(_self.copyWith(field0: value));
    });
  }
}

// dart format on
