// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'remote.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$BridgeRemoteFit {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeRemoteFit);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeRemoteFit()';
  }
}

/// @nodoc
class $BridgeRemoteFitCopyWith<$Res> {
  $BridgeRemoteFitCopyWith(
      BridgeRemoteFit _, $Res Function(BridgeRemoteFit) __);
}

/// Adds pattern-matching-related methods to [BridgeRemoteFit].
extension BridgeRemoteFitPatterns on BridgeRemoteFit {
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
    TResult Function(BridgeRemoteFit_Checking value)? checking,
    TResult Function(BridgeRemoteFit_Ready value)? ready,
    TResult Function(BridgeRemoteFit_Lacks value)? lacks,
    TResult Function(BridgeRemoteFit_TooOld value)? tooOld,
    TResult Function(BridgeRemoteFit_Unknown value)? unknown,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeRemoteFit_Checking() when checking != null:
        return checking(_that);
      case BridgeRemoteFit_Ready() when ready != null:
        return ready(_that);
      case BridgeRemoteFit_Lacks() when lacks != null:
        return lacks(_that);
      case BridgeRemoteFit_TooOld() when tooOld != null:
        return tooOld(_that);
      case BridgeRemoteFit_Unknown() when unknown != null:
        return unknown(_that);
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
    required TResult Function(BridgeRemoteFit_Checking value) checking,
    required TResult Function(BridgeRemoteFit_Ready value) ready,
    required TResult Function(BridgeRemoteFit_Lacks value) lacks,
    required TResult Function(BridgeRemoteFit_TooOld value) tooOld,
    required TResult Function(BridgeRemoteFit_Unknown value) unknown,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeRemoteFit_Checking():
        return checking(_that);
      case BridgeRemoteFit_Ready():
        return ready(_that);
      case BridgeRemoteFit_Lacks():
        return lacks(_that);
      case BridgeRemoteFit_TooOld():
        return tooOld(_that);
      case BridgeRemoteFit_Unknown():
        return unknown(_that);
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
    TResult? Function(BridgeRemoteFit_Checking value)? checking,
    TResult? Function(BridgeRemoteFit_Ready value)? ready,
    TResult? Function(BridgeRemoteFit_Lacks value)? lacks,
    TResult? Function(BridgeRemoteFit_TooOld value)? tooOld,
    TResult? Function(BridgeRemoteFit_Unknown value)? unknown,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeRemoteFit_Checking() when checking != null:
        return checking(_that);
      case BridgeRemoteFit_Ready() when ready != null:
        return ready(_that);
      case BridgeRemoteFit_Lacks() when lacks != null:
        return lacks(_that);
      case BridgeRemoteFit_TooOld() when tooOld != null:
        return tooOld(_that);
      case BridgeRemoteFit_Unknown() when unknown != null:
        return unknown(_that);
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
    TResult Function()? checking,
    TResult Function()? ready,
    TResult Function(
            List<String> plugins, List<String> fonts, List<String> models)?
        lacks,
    TResult Function(String needs)? tooOld,
    TResult Function()? unknown,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeRemoteFit_Checking() when checking != null:
        return checking();
      case BridgeRemoteFit_Ready() when ready != null:
        return ready();
      case BridgeRemoteFit_Lacks() when lacks != null:
        return lacks(_that.plugins, _that.fonts, _that.models);
      case BridgeRemoteFit_TooOld() when tooOld != null:
        return tooOld(_that.needs);
      case BridgeRemoteFit_Unknown() when unknown != null:
        return unknown();
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
    required TResult Function() checking,
    required TResult Function() ready,
    required TResult Function(
            List<String> plugins, List<String> fonts, List<String> models)
        lacks,
    required TResult Function(String needs) tooOld,
    required TResult Function() unknown,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeRemoteFit_Checking():
        return checking();
      case BridgeRemoteFit_Ready():
        return ready();
      case BridgeRemoteFit_Lacks():
        return lacks(_that.plugins, _that.fonts, _that.models);
      case BridgeRemoteFit_TooOld():
        return tooOld(_that.needs);
      case BridgeRemoteFit_Unknown():
        return unknown();
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
    TResult? Function()? checking,
    TResult? Function()? ready,
    TResult? Function(
            List<String> plugins, List<String> fonts, List<String> models)?
        lacks,
    TResult? Function(String needs)? tooOld,
    TResult? Function()? unknown,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeRemoteFit_Checking() when checking != null:
        return checking();
      case BridgeRemoteFit_Ready() when ready != null:
        return ready();
      case BridgeRemoteFit_Lacks() when lacks != null:
        return lacks(_that.plugins, _that.fonts, _that.models);
      case BridgeRemoteFit_TooOld() when tooOld != null:
        return tooOld(_that.needs);
      case BridgeRemoteFit_Unknown() when unknown != null:
        return unknown();
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeRemoteFit_Checking extends BridgeRemoteFit {
  const BridgeRemoteFit_Checking() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeRemoteFit_Checking);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeRemoteFit.checking()';
  }
}

/// @nodoc

class BridgeRemoteFit_Ready extends BridgeRemoteFit {
  const BridgeRemoteFit_Ready() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeRemoteFit_Ready);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeRemoteFit.ready()';
  }
}

/// @nodoc

class BridgeRemoteFit_Lacks extends BridgeRemoteFit {
  const BridgeRemoteFit_Lacks(
      {required final List<String> plugins,
      required final List<String> fonts,
      required final List<String> models})
      : _plugins = plugins,
        _fonts = fonts,
        _models = models,
        super._();

  final List<String> _plugins;
  List<String> get plugins {
    if (_plugins is EqualUnmodifiableListView) return _plugins;
    // ignore: implicit_dynamic_type
    return EqualUnmodifiableListView(_plugins);
  }

  final List<String> _fonts;
  List<String> get fonts {
    if (_fonts is EqualUnmodifiableListView) return _fonts;
    // ignore: implicit_dynamic_type
    return EqualUnmodifiableListView(_fonts);
  }

  final List<String> _models;
  List<String> get models {
    if (_models is EqualUnmodifiableListView) return _models;
    // ignore: implicit_dynamic_type
    return EqualUnmodifiableListView(_models);
  }

  /// Create a copy of BridgeRemoteFit
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeRemoteFit_LacksCopyWith<BridgeRemoteFit_Lacks> get copyWith =>
      _$BridgeRemoteFit_LacksCopyWithImpl<BridgeRemoteFit_Lacks>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeRemoteFit_Lacks &&
            const DeepCollectionEquality().equals(other._plugins, _plugins) &&
            const DeepCollectionEquality().equals(other._fonts, _fonts) &&
            const DeepCollectionEquality().equals(other._models, _models));
  }

  @override
  int get hashCode => Object.hash(
      runtimeType,
      const DeepCollectionEquality().hash(_plugins),
      const DeepCollectionEquality().hash(_fonts),
      const DeepCollectionEquality().hash(_models));

  @override
  String toString() {
    return 'BridgeRemoteFit.lacks(plugins: $plugins, fonts: $fonts, models: $models)';
  }
}

/// @nodoc
abstract mixin class $BridgeRemoteFit_LacksCopyWith<$Res>
    implements $BridgeRemoteFitCopyWith<$Res> {
  factory $BridgeRemoteFit_LacksCopyWith(BridgeRemoteFit_Lacks value,
          $Res Function(BridgeRemoteFit_Lacks) _then) =
      _$BridgeRemoteFit_LacksCopyWithImpl;
  @useResult
  $Res call({List<String> plugins, List<String> fonts, List<String> models});
}

/// @nodoc
class _$BridgeRemoteFit_LacksCopyWithImpl<$Res>
    implements $BridgeRemoteFit_LacksCopyWith<$Res> {
  _$BridgeRemoteFit_LacksCopyWithImpl(this._self, this._then);

  final BridgeRemoteFit_Lacks _self;
  final $Res Function(BridgeRemoteFit_Lacks) _then;

  /// Create a copy of BridgeRemoteFit
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? plugins = null,
    Object? fonts = null,
    Object? models = null,
  }) {
    return _then(BridgeRemoteFit_Lacks(
      plugins: null == plugins
          ? _self._plugins
          : plugins // ignore: cast_nullable_to_non_nullable
              as List<String>,
      fonts: null == fonts
          ? _self._fonts
          : fonts // ignore: cast_nullable_to_non_nullable
              as List<String>,
      models: null == models
          ? _self._models
          : models // ignore: cast_nullable_to_non_nullable
              as List<String>,
    ));
  }
}

/// @nodoc

class BridgeRemoteFit_TooOld extends BridgeRemoteFit {
  const BridgeRemoteFit_TooOld({required this.needs}) : super._();

  final String needs;

  /// Create a copy of BridgeRemoteFit
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeRemoteFit_TooOldCopyWith<BridgeRemoteFit_TooOld> get copyWith =>
      _$BridgeRemoteFit_TooOldCopyWithImpl<BridgeRemoteFit_TooOld>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeRemoteFit_TooOld &&
            (identical(other.needs, needs) || other.needs == needs));
  }

  @override
  int get hashCode => Object.hash(runtimeType, needs);

  @override
  String toString() {
    return 'BridgeRemoteFit.tooOld(needs: $needs)';
  }
}

/// @nodoc
abstract mixin class $BridgeRemoteFit_TooOldCopyWith<$Res>
    implements $BridgeRemoteFitCopyWith<$Res> {
  factory $BridgeRemoteFit_TooOldCopyWith(BridgeRemoteFit_TooOld value,
          $Res Function(BridgeRemoteFit_TooOld) _then) =
      _$BridgeRemoteFit_TooOldCopyWithImpl;
  @useResult
  $Res call({String needs});
}

/// @nodoc
class _$BridgeRemoteFit_TooOldCopyWithImpl<$Res>
    implements $BridgeRemoteFit_TooOldCopyWith<$Res> {
  _$BridgeRemoteFit_TooOldCopyWithImpl(this._self, this._then);

  final BridgeRemoteFit_TooOld _self;
  final $Res Function(BridgeRemoteFit_TooOld) _then;

  /// Create a copy of BridgeRemoteFit
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? needs = null,
  }) {
    return _then(BridgeRemoteFit_TooOld(
      needs: null == needs
          ? _self.needs
          : needs // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeRemoteFit_Unknown extends BridgeRemoteFit {
  const BridgeRemoteFit_Unknown() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeRemoteFit_Unknown);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeRemoteFit.unknown()';
  }
}

// dart format on
