// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'graph.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$BridgeInputRef {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeInputRef);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeInputRef()';
  }
}

/// @nodoc
class $BridgeInputRefCopyWith<$Res> {
  $BridgeInputRefCopyWith(BridgeInputRef _, $Res Function(BridgeInputRef) __);
}

/// Adds pattern-matching-related methods to [BridgeInputRef].
extension BridgeInputRefPatterns on BridgeInputRef {
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
    TResult Function(BridgeInputRef_Param value)? param,
    TResult Function(BridgeInputRef_Matte value)? matte,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeInputRef_Param() when param != null:
        return param(_that);
      case BridgeInputRef_Matte() when matte != null:
        return matte(_that);
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
    required TResult Function(BridgeInputRef_Param value) param,
    required TResult Function(BridgeInputRef_Matte value) matte,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeInputRef_Param():
        return param(_that);
      case BridgeInputRef_Matte():
        return matte(_that);
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
    TResult? Function(BridgeInputRef_Param value)? param,
    TResult? Function(BridgeInputRef_Matte value)? matte,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeInputRef_Param() when param != null:
        return param(_that);
      case BridgeInputRef_Matte() when matte != null:
        return matte(_that);
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
    TResult Function(BridgeNodeRef node, String port)? param,
    TResult Function(UuidValue effect)? matte,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeInputRef_Param() when param != null:
        return param(_that.node, _that.port);
      case BridgeInputRef_Matte() when matte != null:
        return matte(_that.effect);
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
    required TResult Function(BridgeNodeRef node, String port) param,
    required TResult Function(UuidValue effect) matte,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeInputRef_Param():
        return param(_that.node, _that.port);
      case BridgeInputRef_Matte():
        return matte(_that.effect);
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
    TResult? Function(BridgeNodeRef node, String port)? param,
    TResult? Function(UuidValue effect)? matte,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeInputRef_Param() when param != null:
        return param(_that.node, _that.port);
      case BridgeInputRef_Matte() when matte != null:
        return matte(_that.effect);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeInputRef_Param extends BridgeInputRef {
  const BridgeInputRef_Param({required this.node, required this.port})
      : super._();

  final BridgeNodeRef node;
  final String port;

  /// Create a copy of BridgeInputRef
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeInputRef_ParamCopyWith<BridgeInputRef_Param> get copyWith =>
      _$BridgeInputRef_ParamCopyWithImpl<BridgeInputRef_Param>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeInputRef_Param &&
            (identical(other.node, node) || other.node == node) &&
            (identical(other.port, port) || other.port == port));
  }

  @override
  int get hashCode => Object.hash(runtimeType, node, port);

  @override
  String toString() {
    return 'BridgeInputRef.param(node: $node, port: $port)';
  }
}

/// @nodoc
abstract mixin class $BridgeInputRef_ParamCopyWith<$Res>
    implements $BridgeInputRefCopyWith<$Res> {
  factory $BridgeInputRef_ParamCopyWith(BridgeInputRef_Param value,
          $Res Function(BridgeInputRef_Param) _then) =
      _$BridgeInputRef_ParamCopyWithImpl;
  @useResult
  $Res call({BridgeNodeRef node, String port});

  $BridgeNodeRefCopyWith<$Res> get node;
}

/// @nodoc
class _$BridgeInputRef_ParamCopyWithImpl<$Res>
    implements $BridgeInputRef_ParamCopyWith<$Res> {
  _$BridgeInputRef_ParamCopyWithImpl(this._self, this._then);

  final BridgeInputRef_Param _self;
  final $Res Function(BridgeInputRef_Param) _then;

  /// Create a copy of BridgeInputRef
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? node = null,
    Object? port = null,
  }) {
    return _then(BridgeInputRef_Param(
      node: null == node
          ? _self.node
          : node // ignore: cast_nullable_to_non_nullable
              as BridgeNodeRef,
      port: null == port
          ? _self.port
          : port // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }

  /// Create a copy of BridgeInputRef
  /// with the given fields replaced by the non-null parameter values.
  @override
  @pragma('vm:prefer-inline')
  $BridgeNodeRefCopyWith<$Res> get node {
    return $BridgeNodeRefCopyWith<$Res>(_self.node, (value) {
      return _then(_self.copyWith(node: value));
    });
  }
}

/// @nodoc

class BridgeInputRef_Matte extends BridgeInputRef {
  const BridgeInputRef_Matte({required this.effect}) : super._();

  final UuidValue effect;

  /// Create a copy of BridgeInputRef
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeInputRef_MatteCopyWith<BridgeInputRef_Matte> get copyWith =>
      _$BridgeInputRef_MatteCopyWithImpl<BridgeInputRef_Matte>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeInputRef_Matte &&
            (identical(other.effect, effect) || other.effect == effect));
  }

  @override
  int get hashCode => Object.hash(runtimeType, effect);

  @override
  String toString() {
    return 'BridgeInputRef.matte(effect: $effect)';
  }
}

/// @nodoc
abstract mixin class $BridgeInputRef_MatteCopyWith<$Res>
    implements $BridgeInputRefCopyWith<$Res> {
  factory $BridgeInputRef_MatteCopyWith(BridgeInputRef_Matte value,
          $Res Function(BridgeInputRef_Matte) _then) =
      _$BridgeInputRef_MatteCopyWithImpl;
  @useResult
  $Res call({UuidValue effect});
}

/// @nodoc
class _$BridgeInputRef_MatteCopyWithImpl<$Res>
    implements $BridgeInputRef_MatteCopyWith<$Res> {
  _$BridgeInputRef_MatteCopyWithImpl(this._self, this._then);

  final BridgeInputRef_Matte _self;
  final $Res Function(BridgeInputRef_Matte) _then;

  /// Create a copy of BridgeInputRef
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? effect = null,
  }) {
    return _then(BridgeInputRef_Matte(
      effect: null == effect
          ? _self.effect
          : effect // ignore: cast_nullable_to_non_nullable
              as UuidValue,
    ));
  }
}

/// @nodoc
mixin _$BridgeNodeRef {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeNodeRef);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeNodeRef()';
  }
}

/// @nodoc
class $BridgeNodeRefCopyWith<$Res> {
  $BridgeNodeRefCopyWith(BridgeNodeRef _, $Res Function(BridgeNodeRef) __);
}

/// Adds pattern-matching-related methods to [BridgeNodeRef].
extension BridgeNodeRefPatterns on BridgeNodeRef {
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
    TResult Function(BridgeNodeRef_Source value)? source,
    TResult Function(BridgeNodeRef_Effect value)? effect,
    TResult Function(BridgeNodeRef_Driver value)? driver,
    TResult Function(BridgeNodeRef_Out value)? out,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeNodeRef_Source() when source != null:
        return source(_that);
      case BridgeNodeRef_Effect() when effect != null:
        return effect(_that);
      case BridgeNodeRef_Driver() when driver != null:
        return driver(_that);
      case BridgeNodeRef_Out() when out != null:
        return out(_that);
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
    required TResult Function(BridgeNodeRef_Source value) source,
    required TResult Function(BridgeNodeRef_Effect value) effect,
    required TResult Function(BridgeNodeRef_Driver value) driver,
    required TResult Function(BridgeNodeRef_Out value) out,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeNodeRef_Source():
        return source(_that);
      case BridgeNodeRef_Effect():
        return effect(_that);
      case BridgeNodeRef_Driver():
        return driver(_that);
      case BridgeNodeRef_Out():
        return out(_that);
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
    TResult? Function(BridgeNodeRef_Source value)? source,
    TResult? Function(BridgeNodeRef_Effect value)? effect,
    TResult? Function(BridgeNodeRef_Driver value)? driver,
    TResult? Function(BridgeNodeRef_Out value)? out,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeNodeRef_Source() when source != null:
        return source(_that);
      case BridgeNodeRef_Effect() when effect != null:
        return effect(_that);
      case BridgeNodeRef_Driver() when driver != null:
        return driver(_that);
      case BridgeNodeRef_Out() when out != null:
        return out(_that);
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
    TResult Function()? source,
    TResult Function(UuidValue field0)? effect,
    TResult Function(UuidValue field0)? driver,
    TResult Function()? out,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeNodeRef_Source() when source != null:
        return source();
      case BridgeNodeRef_Effect() when effect != null:
        return effect(_that.field0);
      case BridgeNodeRef_Driver() when driver != null:
        return driver(_that.field0);
      case BridgeNodeRef_Out() when out != null:
        return out();
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
    required TResult Function() source,
    required TResult Function(UuidValue field0) effect,
    required TResult Function(UuidValue field0) driver,
    required TResult Function() out,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeNodeRef_Source():
        return source();
      case BridgeNodeRef_Effect():
        return effect(_that.field0);
      case BridgeNodeRef_Driver():
        return driver(_that.field0);
      case BridgeNodeRef_Out():
        return out();
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
    TResult? Function()? source,
    TResult? Function(UuidValue field0)? effect,
    TResult? Function(UuidValue field0)? driver,
    TResult? Function()? out,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeNodeRef_Source() when source != null:
        return source();
      case BridgeNodeRef_Effect() when effect != null:
        return effect(_that.field0);
      case BridgeNodeRef_Driver() when driver != null:
        return driver(_that.field0);
      case BridgeNodeRef_Out() when out != null:
        return out();
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeNodeRef_Source extends BridgeNodeRef {
  const BridgeNodeRef_Source() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeNodeRef_Source);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeNodeRef.source()';
  }
}

/// @nodoc

class BridgeNodeRef_Effect extends BridgeNodeRef {
  const BridgeNodeRef_Effect(this.field0) : super._();

  final UuidValue field0;

  /// Create a copy of BridgeNodeRef
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeNodeRef_EffectCopyWith<BridgeNodeRef_Effect> get copyWith =>
      _$BridgeNodeRef_EffectCopyWithImpl<BridgeNodeRef_Effect>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeNodeRef_Effect &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeNodeRef.effect(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeNodeRef_EffectCopyWith<$Res>
    implements $BridgeNodeRefCopyWith<$Res> {
  factory $BridgeNodeRef_EffectCopyWith(BridgeNodeRef_Effect value,
          $Res Function(BridgeNodeRef_Effect) _then) =
      _$BridgeNodeRef_EffectCopyWithImpl;
  @useResult
  $Res call({UuidValue field0});
}

/// @nodoc
class _$BridgeNodeRef_EffectCopyWithImpl<$Res>
    implements $BridgeNodeRef_EffectCopyWith<$Res> {
  _$BridgeNodeRef_EffectCopyWithImpl(this._self, this._then);

  final BridgeNodeRef_Effect _self;
  final $Res Function(BridgeNodeRef_Effect) _then;

  /// Create a copy of BridgeNodeRef
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeNodeRef_Effect(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as UuidValue,
    ));
  }
}

/// @nodoc

class BridgeNodeRef_Driver extends BridgeNodeRef {
  const BridgeNodeRef_Driver(this.field0) : super._();

  final UuidValue field0;

  /// Create a copy of BridgeNodeRef
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeNodeRef_DriverCopyWith<BridgeNodeRef_Driver> get copyWith =>
      _$BridgeNodeRef_DriverCopyWithImpl<BridgeNodeRef_Driver>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeNodeRef_Driver &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'BridgeNodeRef.driver(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $BridgeNodeRef_DriverCopyWith<$Res>
    implements $BridgeNodeRefCopyWith<$Res> {
  factory $BridgeNodeRef_DriverCopyWith(BridgeNodeRef_Driver value,
          $Res Function(BridgeNodeRef_Driver) _then) =
      _$BridgeNodeRef_DriverCopyWithImpl;
  @useResult
  $Res call({UuidValue field0});
}

/// @nodoc
class _$BridgeNodeRef_DriverCopyWithImpl<$Res>
    implements $BridgeNodeRef_DriverCopyWith<$Res> {
  _$BridgeNodeRef_DriverCopyWithImpl(this._self, this._then);

  final BridgeNodeRef_Driver _self;
  final $Res Function(BridgeNodeRef_Driver) _then;

  /// Create a copy of BridgeNodeRef
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(BridgeNodeRef_Driver(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as UuidValue,
    ));
  }
}

/// @nodoc

class BridgeNodeRef_Out extends BridgeNodeRef {
  const BridgeNodeRef_Out() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeNodeRef_Out);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeNodeRef.out()';
  }
}

/// @nodoc
mixin _$BridgeOutputRef {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeOutputRef);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeOutputRef()';
  }
}

/// @nodoc
class $BridgeOutputRefCopyWith<$Res> {
  $BridgeOutputRefCopyWith(
      BridgeOutputRef _, $Res Function(BridgeOutputRef) __);
}

/// Adds pattern-matching-related methods to [BridgeOutputRef].
extension BridgeOutputRefPatterns on BridgeOutputRef {
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
    TResult Function(BridgeOutputRef_Driver value)? driver,
    TResult Function(BridgeOutputRef_SourceMatte value)? sourceMatte,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOutputRef_Driver() when driver != null:
        return driver(_that);
      case BridgeOutputRef_SourceMatte() when sourceMatte != null:
        return sourceMatte(_that);
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
    required TResult Function(BridgeOutputRef_Driver value) driver,
    required TResult Function(BridgeOutputRef_SourceMatte value) sourceMatte,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOutputRef_Driver():
        return driver(_that);
      case BridgeOutputRef_SourceMatte():
        return sourceMatte(_that);
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
    TResult? Function(BridgeOutputRef_Driver value)? driver,
    TResult? Function(BridgeOutputRef_SourceMatte value)? sourceMatte,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOutputRef_Driver() when driver != null:
        return driver(_that);
      case BridgeOutputRef_SourceMatte() when sourceMatte != null:
        return sourceMatte(_that);
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
    TResult Function(UuidValue node, String port)? driver,
    TResult Function()? sourceMatte,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOutputRef_Driver() when driver != null:
        return driver(_that.node, _that.port);
      case BridgeOutputRef_SourceMatte() when sourceMatte != null:
        return sourceMatte();
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
    required TResult Function(UuidValue node, String port) driver,
    required TResult Function() sourceMatte,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOutputRef_Driver():
        return driver(_that.node, _that.port);
      case BridgeOutputRef_SourceMatte():
        return sourceMatte();
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
    TResult? Function(UuidValue node, String port)? driver,
    TResult? Function()? sourceMatte,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOutputRef_Driver() when driver != null:
        return driver(_that.node, _that.port);
      case BridgeOutputRef_SourceMatte() when sourceMatte != null:
        return sourceMatte();
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeOutputRef_Driver extends BridgeOutputRef {
  const BridgeOutputRef_Driver({required this.node, required this.port})
      : super._();

  final UuidValue node;
  final String port;

  /// Create a copy of BridgeOutputRef
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeOutputRef_DriverCopyWith<BridgeOutputRef_Driver> get copyWith =>
      _$BridgeOutputRef_DriverCopyWithImpl<BridgeOutputRef_Driver>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeOutputRef_Driver &&
            (identical(other.node, node) || other.node == node) &&
            (identical(other.port, port) || other.port == port));
  }

  @override
  int get hashCode => Object.hash(runtimeType, node, port);

  @override
  String toString() {
    return 'BridgeOutputRef.driver(node: $node, port: $port)';
  }
}

/// @nodoc
abstract mixin class $BridgeOutputRef_DriverCopyWith<$Res>
    implements $BridgeOutputRefCopyWith<$Res> {
  factory $BridgeOutputRef_DriverCopyWith(BridgeOutputRef_Driver value,
          $Res Function(BridgeOutputRef_Driver) _then) =
      _$BridgeOutputRef_DriverCopyWithImpl;
  @useResult
  $Res call({UuidValue node, String port});
}

/// @nodoc
class _$BridgeOutputRef_DriverCopyWithImpl<$Res>
    implements $BridgeOutputRef_DriverCopyWith<$Res> {
  _$BridgeOutputRef_DriverCopyWithImpl(this._self, this._then);

  final BridgeOutputRef_Driver _self;
  final $Res Function(BridgeOutputRef_Driver) _then;

  /// Create a copy of BridgeOutputRef
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? node = null,
    Object? port = null,
  }) {
    return _then(BridgeOutputRef_Driver(
      node: null == node
          ? _self.node
          : node // ignore: cast_nullable_to_non_nullable
              as UuidValue,
      port: null == port
          ? _self.port
          : port // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeOutputRef_SourceMatte extends BridgeOutputRef {
  const BridgeOutputRef_SourceMatte() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeOutputRef_SourceMatte);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeOutputRef.sourceMatte()';
  }
}

// dart format on
