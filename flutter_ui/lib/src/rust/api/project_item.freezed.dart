// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'project_item.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$ItemReference {
  Object get field0;

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is ItemReference &&
            const DeepCollectionEquality().equals(other.field0, field0));
  }

  @override
  int get hashCode =>
      Object.hash(runtimeType, const DeepCollectionEquality().hash(field0));

  @override
  String toString() {
    return 'ItemReference(field0: $field0)';
  }
}

/// @nodoc
class $ItemReferenceCopyWith<$Res> {
  $ItemReferenceCopyWith(ItemReference _, $Res Function(ItemReference) __);
}

/// Adds pattern-matching-related methods to [ItemReference].
extension ItemReferencePatterns on ItemReference {
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
    TResult Function(ItemReference_Footage value)? footage,
    TResult Function(ItemReference_Solid value)? solid,
    TResult Function(ItemReference_Composition value)? composition,
    TResult Function(ItemReference_Folder value)? folder,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case ItemReference_Footage() when footage != null:
        return footage(_that);
      case ItemReference_Solid() when solid != null:
        return solid(_that);
      case ItemReference_Composition() when composition != null:
        return composition(_that);
      case ItemReference_Folder() when folder != null:
        return folder(_that);
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
    required TResult Function(ItemReference_Footage value) footage,
    required TResult Function(ItemReference_Solid value) solid,
    required TResult Function(ItemReference_Composition value) composition,
    required TResult Function(ItemReference_Folder value) folder,
  }) {
    final _that = this;
    switch (_that) {
      case ItemReference_Footage():
        return footage(_that);
      case ItemReference_Solid():
        return solid(_that);
      case ItemReference_Composition():
        return composition(_that);
      case ItemReference_Folder():
        return folder(_that);
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
    TResult? Function(ItemReference_Footage value)? footage,
    TResult? Function(ItemReference_Solid value)? solid,
    TResult? Function(ItemReference_Composition value)? composition,
    TResult? Function(ItemReference_Folder value)? folder,
  }) {
    final _that = this;
    switch (_that) {
      case ItemReference_Footage() when footage != null:
        return footage(_that);
      case ItemReference_Solid() when solid != null:
        return solid(_that);
      case ItemReference_Composition() when composition != null:
        return composition(_that);
      case ItemReference_Folder() when folder != null:
        return folder(_that);
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
    TResult Function(FootageReference field0)? footage,
    TResult Function(SolidReference field0)? solid,
    TResult Function(CompositionReference field0)? composition,
    TResult Function(FolderReference field0)? folder,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case ItemReference_Footage() when footage != null:
        return footage(_that.field0);
      case ItemReference_Solid() when solid != null:
        return solid(_that.field0);
      case ItemReference_Composition() when composition != null:
        return composition(_that.field0);
      case ItemReference_Folder() when folder != null:
        return folder(_that.field0);
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
    required TResult Function(FootageReference field0) footage,
    required TResult Function(SolidReference field0) solid,
    required TResult Function(CompositionReference field0) composition,
    required TResult Function(FolderReference field0) folder,
  }) {
    final _that = this;
    switch (_that) {
      case ItemReference_Footage():
        return footage(_that.field0);
      case ItemReference_Solid():
        return solid(_that.field0);
      case ItemReference_Composition():
        return composition(_that.field0);
      case ItemReference_Folder():
        return folder(_that.field0);
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
    TResult? Function(FootageReference field0)? footage,
    TResult? Function(SolidReference field0)? solid,
    TResult? Function(CompositionReference field0)? composition,
    TResult? Function(FolderReference field0)? folder,
  }) {
    final _that = this;
    switch (_that) {
      case ItemReference_Footage() when footage != null:
        return footage(_that.field0);
      case ItemReference_Solid() when solid != null:
        return solid(_that.field0);
      case ItemReference_Composition() when composition != null:
        return composition(_that.field0);
      case ItemReference_Folder() when folder != null:
        return folder(_that.field0);
      case _:
        return null;
    }
  }
}

/// @nodoc

class ItemReference_Footage extends ItemReference {
  const ItemReference_Footage(this.field0) : super._();

  @override
  final FootageReference field0;

  /// Create a copy of ItemReference
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $ItemReference_FootageCopyWith<ItemReference_Footage> get copyWith =>
      _$ItemReference_FootageCopyWithImpl<ItemReference_Footage>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is ItemReference_Footage &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'ItemReference.footage(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $ItemReference_FootageCopyWith<$Res>
    implements $ItemReferenceCopyWith<$Res> {
  factory $ItemReference_FootageCopyWith(ItemReference_Footage value,
          $Res Function(ItemReference_Footage) _then) =
      _$ItemReference_FootageCopyWithImpl;
  @useResult
  $Res call({FootageReference field0});
}

/// @nodoc
class _$ItemReference_FootageCopyWithImpl<$Res>
    implements $ItemReference_FootageCopyWith<$Res> {
  _$ItemReference_FootageCopyWithImpl(this._self, this._then);

  final ItemReference_Footage _self;
  final $Res Function(ItemReference_Footage) _then;

  /// Create a copy of ItemReference
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(ItemReference_Footage(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as FootageReference,
    ));
  }
}

/// @nodoc

class ItemReference_Solid extends ItemReference {
  const ItemReference_Solid(this.field0) : super._();

  @override
  final SolidReference field0;

  /// Create a copy of ItemReference
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $ItemReference_SolidCopyWith<ItemReference_Solid> get copyWith =>
      _$ItemReference_SolidCopyWithImpl<ItemReference_Solid>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is ItemReference_Solid &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'ItemReference.solid(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $ItemReference_SolidCopyWith<$Res>
    implements $ItemReferenceCopyWith<$Res> {
  factory $ItemReference_SolidCopyWith(
          ItemReference_Solid value, $Res Function(ItemReference_Solid) _then) =
      _$ItemReference_SolidCopyWithImpl;
  @useResult
  $Res call({SolidReference field0});
}

/// @nodoc
class _$ItemReference_SolidCopyWithImpl<$Res>
    implements $ItemReference_SolidCopyWith<$Res> {
  _$ItemReference_SolidCopyWithImpl(this._self, this._then);

  final ItemReference_Solid _self;
  final $Res Function(ItemReference_Solid) _then;

  /// Create a copy of ItemReference
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(ItemReference_Solid(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as SolidReference,
    ));
  }
}

/// @nodoc

class ItemReference_Composition extends ItemReference {
  const ItemReference_Composition(this.field0) : super._();

  @override
  final CompositionReference field0;

  /// Create a copy of ItemReference
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $ItemReference_CompositionCopyWith<ItemReference_Composition> get copyWith =>
      _$ItemReference_CompositionCopyWithImpl<ItemReference_Composition>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is ItemReference_Composition &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'ItemReference.composition(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $ItemReference_CompositionCopyWith<$Res>
    implements $ItemReferenceCopyWith<$Res> {
  factory $ItemReference_CompositionCopyWith(ItemReference_Composition value,
          $Res Function(ItemReference_Composition) _then) =
      _$ItemReference_CompositionCopyWithImpl;
  @useResult
  $Res call({CompositionReference field0});
}

/// @nodoc
class _$ItemReference_CompositionCopyWithImpl<$Res>
    implements $ItemReference_CompositionCopyWith<$Res> {
  _$ItemReference_CompositionCopyWithImpl(this._self, this._then);

  final ItemReference_Composition _self;
  final $Res Function(ItemReference_Composition) _then;

  /// Create a copy of ItemReference
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(ItemReference_Composition(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as CompositionReference,
    ));
  }
}

/// @nodoc

class ItemReference_Folder extends ItemReference {
  const ItemReference_Folder(this.field0) : super._();

  @override
  final FolderReference field0;

  /// Create a copy of ItemReference
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $ItemReference_FolderCopyWith<ItemReference_Folder> get copyWith =>
      _$ItemReference_FolderCopyWithImpl<ItemReference_Folder>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is ItemReference_Folder &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'ItemReference.folder(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $ItemReference_FolderCopyWith<$Res>
    implements $ItemReferenceCopyWith<$Res> {
  factory $ItemReference_FolderCopyWith(ItemReference_Folder value,
          $Res Function(ItemReference_Folder) _then) =
      _$ItemReference_FolderCopyWithImpl;
  @useResult
  $Res call({FolderReference field0});
}

/// @nodoc
class _$ItemReference_FolderCopyWithImpl<$Res>
    implements $ItemReference_FolderCopyWith<$Res> {
  _$ItemReference_FolderCopyWithImpl(this._self, this._then);

  final ItemReference_Folder _self;
  final $Res Function(ItemReference_Folder) _then;

  /// Create a copy of ItemReference
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(ItemReference_Folder(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as FolderReference,
    ));
  }
}

// dart format on
