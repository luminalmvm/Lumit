// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'script.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$BridgeLiveEvent {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeLiveEvent);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeLiveEvent()';
  }
}

/// @nodoc
class $BridgeLiveEventCopyWith<$Res> {
  $BridgeLiveEventCopyWith(
      BridgeLiveEvent _, $Res Function(BridgeLiveEvent) __);
}

/// Adds pattern-matching-related methods to [BridgeLiveEvent].
extension BridgeLiveEventPatterns on BridgeLiveEvent {
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
    TResult Function(BridgeLiveEvent_Attached value)? attached,
    TResult Function(BridgeLiveEvent_Detached value)? detached,
    TResult Function(BridgeLiveEvent_ScriptStarted value)? scriptStarted,
    TResult Function(BridgeLiveEvent_ScriptEnded value)? scriptEnded,
    TResult Function(BridgeLiveEvent_OpenComp value)? openComp,
    TResult Function(BridgeLiveEvent_Alert value)? alert,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeLiveEvent_Attached() when attached != null:
        return attached(_that);
      case BridgeLiveEvent_Detached() when detached != null:
        return detached(_that);
      case BridgeLiveEvent_ScriptStarted() when scriptStarted != null:
        return scriptStarted(_that);
      case BridgeLiveEvent_ScriptEnded() when scriptEnded != null:
        return scriptEnded(_that);
      case BridgeLiveEvent_OpenComp() when openComp != null:
        return openComp(_that);
      case BridgeLiveEvent_Alert() when alert != null:
        return alert(_that);
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
    required TResult Function(BridgeLiveEvent_Attached value) attached,
    required TResult Function(BridgeLiveEvent_Detached value) detached,
    required TResult Function(BridgeLiveEvent_ScriptStarted value)
        scriptStarted,
    required TResult Function(BridgeLiveEvent_ScriptEnded value) scriptEnded,
    required TResult Function(BridgeLiveEvent_OpenComp value) openComp,
    required TResult Function(BridgeLiveEvent_Alert value) alert,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeLiveEvent_Attached():
        return attached(_that);
      case BridgeLiveEvent_Detached():
        return detached(_that);
      case BridgeLiveEvent_ScriptStarted():
        return scriptStarted(_that);
      case BridgeLiveEvent_ScriptEnded():
        return scriptEnded(_that);
      case BridgeLiveEvent_OpenComp():
        return openComp(_that);
      case BridgeLiveEvent_Alert():
        return alert(_that);
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
    TResult? Function(BridgeLiveEvent_Attached value)? attached,
    TResult? Function(BridgeLiveEvent_Detached value)? detached,
    TResult? Function(BridgeLiveEvent_ScriptStarted value)? scriptStarted,
    TResult? Function(BridgeLiveEvent_ScriptEnded value)? scriptEnded,
    TResult? Function(BridgeLiveEvent_OpenComp value)? openComp,
    TResult? Function(BridgeLiveEvent_Alert value)? alert,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeLiveEvent_Attached() when attached != null:
        return attached(_that);
      case BridgeLiveEvent_Detached() when detached != null:
        return detached(_that);
      case BridgeLiveEvent_ScriptStarted() when scriptStarted != null:
        return scriptStarted(_that);
      case BridgeLiveEvent_ScriptEnded() when scriptEnded != null:
        return scriptEnded(_that);
      case BridgeLiveEvent_OpenComp() when openComp != null:
        return openComp(_that);
      case BridgeLiveEvent_Alert() when alert != null:
        return alert(_that);
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
    TResult Function(String name)? attached,
    TResult Function()? detached,
    TResult Function(String name)? scriptStarted,
    TResult Function()? scriptEnded,
    TResult Function(CompositionReference comp)? openComp,
    TResult Function(String message)? alert,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeLiveEvent_Attached() when attached != null:
        return attached(_that.name);
      case BridgeLiveEvent_Detached() when detached != null:
        return detached();
      case BridgeLiveEvent_ScriptStarted() when scriptStarted != null:
        return scriptStarted(_that.name);
      case BridgeLiveEvent_ScriptEnded() when scriptEnded != null:
        return scriptEnded();
      case BridgeLiveEvent_OpenComp() when openComp != null:
        return openComp(_that.comp);
      case BridgeLiveEvent_Alert() when alert != null:
        return alert(_that.message);
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
    required TResult Function(String name) attached,
    required TResult Function() detached,
    required TResult Function(String name) scriptStarted,
    required TResult Function() scriptEnded,
    required TResult Function(CompositionReference comp) openComp,
    required TResult Function(String message) alert,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeLiveEvent_Attached():
        return attached(_that.name);
      case BridgeLiveEvent_Detached():
        return detached();
      case BridgeLiveEvent_ScriptStarted():
        return scriptStarted(_that.name);
      case BridgeLiveEvent_ScriptEnded():
        return scriptEnded();
      case BridgeLiveEvent_OpenComp():
        return openComp(_that.comp);
      case BridgeLiveEvent_Alert():
        return alert(_that.message);
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
    TResult? Function(String name)? attached,
    TResult? Function()? detached,
    TResult? Function(String name)? scriptStarted,
    TResult? Function()? scriptEnded,
    TResult? Function(CompositionReference comp)? openComp,
    TResult? Function(String message)? alert,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeLiveEvent_Attached() when attached != null:
        return attached(_that.name);
      case BridgeLiveEvent_Detached() when detached != null:
        return detached();
      case BridgeLiveEvent_ScriptStarted() when scriptStarted != null:
        return scriptStarted(_that.name);
      case BridgeLiveEvent_ScriptEnded() when scriptEnded != null:
        return scriptEnded();
      case BridgeLiveEvent_OpenComp() when openComp != null:
        return openComp(_that.comp);
      case BridgeLiveEvent_Alert() when alert != null:
        return alert(_that.message);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeLiveEvent_Attached extends BridgeLiveEvent {
  const BridgeLiveEvent_Attached({required this.name}) : super._();

  final String name;

  /// Create a copy of BridgeLiveEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeLiveEvent_AttachedCopyWith<BridgeLiveEvent_Attached> get copyWith =>
      _$BridgeLiveEvent_AttachedCopyWithImpl<BridgeLiveEvent_Attached>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeLiveEvent_Attached &&
            (identical(other.name, name) || other.name == name));
  }

  @override
  int get hashCode => Object.hash(runtimeType, name);

  @override
  String toString() {
    return 'BridgeLiveEvent.attached(name: $name)';
  }
}

/// @nodoc
abstract mixin class $BridgeLiveEvent_AttachedCopyWith<$Res>
    implements $BridgeLiveEventCopyWith<$Res> {
  factory $BridgeLiveEvent_AttachedCopyWith(BridgeLiveEvent_Attached value,
          $Res Function(BridgeLiveEvent_Attached) _then) =
      _$BridgeLiveEvent_AttachedCopyWithImpl;
  @useResult
  $Res call({String name});
}

/// @nodoc
class _$BridgeLiveEvent_AttachedCopyWithImpl<$Res>
    implements $BridgeLiveEvent_AttachedCopyWith<$Res> {
  _$BridgeLiveEvent_AttachedCopyWithImpl(this._self, this._then);

  final BridgeLiveEvent_Attached _self;
  final $Res Function(BridgeLiveEvent_Attached) _then;

  /// Create a copy of BridgeLiveEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? name = null,
  }) {
    return _then(BridgeLiveEvent_Attached(
      name: null == name
          ? _self.name
          : name // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeLiveEvent_Detached extends BridgeLiveEvent {
  const BridgeLiveEvent_Detached() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeLiveEvent_Detached);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeLiveEvent.detached()';
  }
}

/// @nodoc

class BridgeLiveEvent_ScriptStarted extends BridgeLiveEvent {
  const BridgeLiveEvent_ScriptStarted({required this.name}) : super._();

  final String name;

  /// Create a copy of BridgeLiveEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeLiveEvent_ScriptStartedCopyWith<BridgeLiveEvent_ScriptStarted>
      get copyWith => _$BridgeLiveEvent_ScriptStartedCopyWithImpl<
          BridgeLiveEvent_ScriptStarted>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeLiveEvent_ScriptStarted &&
            (identical(other.name, name) || other.name == name));
  }

  @override
  int get hashCode => Object.hash(runtimeType, name);

  @override
  String toString() {
    return 'BridgeLiveEvent.scriptStarted(name: $name)';
  }
}

/// @nodoc
abstract mixin class $BridgeLiveEvent_ScriptStartedCopyWith<$Res>
    implements $BridgeLiveEventCopyWith<$Res> {
  factory $BridgeLiveEvent_ScriptStartedCopyWith(
          BridgeLiveEvent_ScriptStarted value,
          $Res Function(BridgeLiveEvent_ScriptStarted) _then) =
      _$BridgeLiveEvent_ScriptStartedCopyWithImpl;
  @useResult
  $Res call({String name});
}

/// @nodoc
class _$BridgeLiveEvent_ScriptStartedCopyWithImpl<$Res>
    implements $BridgeLiveEvent_ScriptStartedCopyWith<$Res> {
  _$BridgeLiveEvent_ScriptStartedCopyWithImpl(this._self, this._then);

  final BridgeLiveEvent_ScriptStarted _self;
  final $Res Function(BridgeLiveEvent_ScriptStarted) _then;

  /// Create a copy of BridgeLiveEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? name = null,
  }) {
    return _then(BridgeLiveEvent_ScriptStarted(
      name: null == name
          ? _self.name
          : name // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeLiveEvent_ScriptEnded extends BridgeLiveEvent {
  const BridgeLiveEvent_ScriptEnded() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeLiveEvent_ScriptEnded);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeLiveEvent.scriptEnded()';
  }
}

/// @nodoc

class BridgeLiveEvent_OpenComp extends BridgeLiveEvent {
  const BridgeLiveEvent_OpenComp({required this.comp}) : super._();

  final CompositionReference comp;

  /// Create a copy of BridgeLiveEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeLiveEvent_OpenCompCopyWith<BridgeLiveEvent_OpenComp> get copyWith =>
      _$BridgeLiveEvent_OpenCompCopyWithImpl<BridgeLiveEvent_OpenComp>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeLiveEvent_OpenComp &&
            (identical(other.comp, comp) || other.comp == comp));
  }

  @override
  int get hashCode => Object.hash(runtimeType, comp);

  @override
  String toString() {
    return 'BridgeLiveEvent.openComp(comp: $comp)';
  }
}

/// @nodoc
abstract mixin class $BridgeLiveEvent_OpenCompCopyWith<$Res>
    implements $BridgeLiveEventCopyWith<$Res> {
  factory $BridgeLiveEvent_OpenCompCopyWith(BridgeLiveEvent_OpenComp value,
          $Res Function(BridgeLiveEvent_OpenComp) _then) =
      _$BridgeLiveEvent_OpenCompCopyWithImpl;
  @useResult
  $Res call({CompositionReference comp});
}

/// @nodoc
class _$BridgeLiveEvent_OpenCompCopyWithImpl<$Res>
    implements $BridgeLiveEvent_OpenCompCopyWith<$Res> {
  _$BridgeLiveEvent_OpenCompCopyWithImpl(this._self, this._then);

  final BridgeLiveEvent_OpenComp _self;
  final $Res Function(BridgeLiveEvent_OpenComp) _then;

  /// Create a copy of BridgeLiveEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? comp = null,
  }) {
    return _then(BridgeLiveEvent_OpenComp(
      comp: null == comp
          ? _self.comp
          : comp // ignore: cast_nullable_to_non_nullable
              as CompositionReference,
    ));
  }
}

/// @nodoc

class BridgeLiveEvent_Alert extends BridgeLiveEvent {
  const BridgeLiveEvent_Alert({required this.message}) : super._();

  final String message;

  /// Create a copy of BridgeLiveEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeLiveEvent_AlertCopyWith<BridgeLiveEvent_Alert> get copyWith =>
      _$BridgeLiveEvent_AlertCopyWithImpl<BridgeLiveEvent_Alert>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeLiveEvent_Alert &&
            (identical(other.message, message) || other.message == message));
  }

  @override
  int get hashCode => Object.hash(runtimeType, message);

  @override
  String toString() {
    return 'BridgeLiveEvent.alert(message: $message)';
  }
}

/// @nodoc
abstract mixin class $BridgeLiveEvent_AlertCopyWith<$Res>
    implements $BridgeLiveEventCopyWith<$Res> {
  factory $BridgeLiveEvent_AlertCopyWith(BridgeLiveEvent_Alert value,
          $Res Function(BridgeLiveEvent_Alert) _then) =
      _$BridgeLiveEvent_AlertCopyWithImpl;
  @useResult
  $Res call({String message});
}

/// @nodoc
class _$BridgeLiveEvent_AlertCopyWithImpl<$Res>
    implements $BridgeLiveEvent_AlertCopyWith<$Res> {
  _$BridgeLiveEvent_AlertCopyWithImpl(this._self, this._then);

  final BridgeLiveEvent_Alert _self;
  final $Res Function(BridgeLiveEvent_Alert) _then;

  /// Create a copy of BridgeLiveEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? message = null,
  }) {
    return _then(BridgeLiveEvent_Alert(
      message: null == message
          ? _self.message
          : message // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc
mixin _$BridgeScriptEvent {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeScriptEvent);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeScriptEvent()';
  }
}

/// @nodoc
class $BridgeScriptEventCopyWith<$Res> {
  $BridgeScriptEventCopyWith(
      BridgeScriptEvent _, $Res Function(BridgeScriptEvent) __);
}

/// Adds pattern-matching-related methods to [BridgeScriptEvent].
extension BridgeScriptEventPatterns on BridgeScriptEvent {
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
    TResult Function(BridgeScriptEvent_ChooseFile value)? chooseFile,
    TResult Function(BridgeScriptEvent_Alert value)? alert,
    TResult Function(BridgeScriptEvent_OpenComp value)? openComp,
    TResult Function(BridgeScriptEvent_Finished value)? finished,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeScriptEvent_ChooseFile() when chooseFile != null:
        return chooseFile(_that);
      case BridgeScriptEvent_Alert() when alert != null:
        return alert(_that);
      case BridgeScriptEvent_OpenComp() when openComp != null:
        return openComp(_that);
      case BridgeScriptEvent_Finished() when finished != null:
        return finished(_that);
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
    required TResult Function(BridgeScriptEvent_ChooseFile value) chooseFile,
    required TResult Function(BridgeScriptEvent_Alert value) alert,
    required TResult Function(BridgeScriptEvent_OpenComp value) openComp,
    required TResult Function(BridgeScriptEvent_Finished value) finished,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeScriptEvent_ChooseFile():
        return chooseFile(_that);
      case BridgeScriptEvent_Alert():
        return alert(_that);
      case BridgeScriptEvent_OpenComp():
        return openComp(_that);
      case BridgeScriptEvent_Finished():
        return finished(_that);
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
    TResult? Function(BridgeScriptEvent_ChooseFile value)? chooseFile,
    TResult? Function(BridgeScriptEvent_Alert value)? alert,
    TResult? Function(BridgeScriptEvent_OpenComp value)? openComp,
    TResult? Function(BridgeScriptEvent_Finished value)? finished,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeScriptEvent_ChooseFile() when chooseFile != null:
        return chooseFile(_that);
      case BridgeScriptEvent_Alert() when alert != null:
        return alert(_that);
      case BridgeScriptEvent_OpenComp() when openComp != null:
        return openComp(_that);
      case BridgeScriptEvent_Finished() when finished != null:
        return finished(_that);
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
    TResult Function(String title, List<String> extensions)? chooseFile,
    TResult Function(String message)? alert,
    TResult Function(CompositionReference comp)? openComp,
    TResult Function(List<String> log, bool stopped, String? error, int? line)?
        finished,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeScriptEvent_ChooseFile() when chooseFile != null:
        return chooseFile(_that.title, _that.extensions);
      case BridgeScriptEvent_Alert() when alert != null:
        return alert(_that.message);
      case BridgeScriptEvent_OpenComp() when openComp != null:
        return openComp(_that.comp);
      case BridgeScriptEvent_Finished() when finished != null:
        return finished(_that.log, _that.stopped, _that.error, _that.line);
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
    required TResult Function(String title, List<String> extensions) chooseFile,
    required TResult Function(String message) alert,
    required TResult Function(CompositionReference comp) openComp,
    required TResult Function(
            List<String> log, bool stopped, String? error, int? line)
        finished,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeScriptEvent_ChooseFile():
        return chooseFile(_that.title, _that.extensions);
      case BridgeScriptEvent_Alert():
        return alert(_that.message);
      case BridgeScriptEvent_OpenComp():
        return openComp(_that.comp);
      case BridgeScriptEvent_Finished():
        return finished(_that.log, _that.stopped, _that.error, _that.line);
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
    TResult? Function(String title, List<String> extensions)? chooseFile,
    TResult? Function(String message)? alert,
    TResult? Function(CompositionReference comp)? openComp,
    TResult? Function(List<String> log, bool stopped, String? error, int? line)?
        finished,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeScriptEvent_ChooseFile() when chooseFile != null:
        return chooseFile(_that.title, _that.extensions);
      case BridgeScriptEvent_Alert() when alert != null:
        return alert(_that.message);
      case BridgeScriptEvent_OpenComp() when openComp != null:
        return openComp(_that.comp);
      case BridgeScriptEvent_Finished() when finished != null:
        return finished(_that.log, _that.stopped, _that.error, _that.line);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeScriptEvent_ChooseFile extends BridgeScriptEvent {
  const BridgeScriptEvent_ChooseFile(
      {required this.title, required final List<String> extensions})
      : _extensions = extensions,
        super._();

  final String title;

  /// The extensions the file may have, without the dot. Empty for any.
  final List<String> _extensions;

  /// The extensions the file may have, without the dot. Empty for any.
  List<String> get extensions {
    if (_extensions is EqualUnmodifiableListView) return _extensions;
    // ignore: implicit_dynamic_type
    return EqualUnmodifiableListView(_extensions);
  }

  /// Create a copy of BridgeScriptEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeScriptEvent_ChooseFileCopyWith<BridgeScriptEvent_ChooseFile>
      get copyWith => _$BridgeScriptEvent_ChooseFileCopyWithImpl<
          BridgeScriptEvent_ChooseFile>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeScriptEvent_ChooseFile &&
            (identical(other.title, title) || other.title == title) &&
            const DeepCollectionEquality()
                .equals(other._extensions, _extensions));
  }

  @override
  int get hashCode => Object.hash(
      runtimeType, title, const DeepCollectionEquality().hash(_extensions));

  @override
  String toString() {
    return 'BridgeScriptEvent.chooseFile(title: $title, extensions: $extensions)';
  }
}

/// @nodoc
abstract mixin class $BridgeScriptEvent_ChooseFileCopyWith<$Res>
    implements $BridgeScriptEventCopyWith<$Res> {
  factory $BridgeScriptEvent_ChooseFileCopyWith(
          BridgeScriptEvent_ChooseFile value,
          $Res Function(BridgeScriptEvent_ChooseFile) _then) =
      _$BridgeScriptEvent_ChooseFileCopyWithImpl;
  @useResult
  $Res call({String title, List<String> extensions});
}

/// @nodoc
class _$BridgeScriptEvent_ChooseFileCopyWithImpl<$Res>
    implements $BridgeScriptEvent_ChooseFileCopyWith<$Res> {
  _$BridgeScriptEvent_ChooseFileCopyWithImpl(this._self, this._then);

  final BridgeScriptEvent_ChooseFile _self;
  final $Res Function(BridgeScriptEvent_ChooseFile) _then;

  /// Create a copy of BridgeScriptEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? title = null,
    Object? extensions = null,
  }) {
    return _then(BridgeScriptEvent_ChooseFile(
      title: null == title
          ? _self.title
          : title // ignore: cast_nullable_to_non_nullable
              as String,
      extensions: null == extensions
          ? _self._extensions
          : extensions // ignore: cast_nullable_to_non_nullable
              as List<String>,
    ));
  }
}

/// @nodoc

class BridgeScriptEvent_Alert extends BridgeScriptEvent {
  const BridgeScriptEvent_Alert({required this.message}) : super._();

  final String message;

  /// Create a copy of BridgeScriptEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeScriptEvent_AlertCopyWith<BridgeScriptEvent_Alert> get copyWith =>
      _$BridgeScriptEvent_AlertCopyWithImpl<BridgeScriptEvent_Alert>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeScriptEvent_Alert &&
            (identical(other.message, message) || other.message == message));
  }

  @override
  int get hashCode => Object.hash(runtimeType, message);

  @override
  String toString() {
    return 'BridgeScriptEvent.alert(message: $message)';
  }
}

/// @nodoc
abstract mixin class $BridgeScriptEvent_AlertCopyWith<$Res>
    implements $BridgeScriptEventCopyWith<$Res> {
  factory $BridgeScriptEvent_AlertCopyWith(BridgeScriptEvent_Alert value,
          $Res Function(BridgeScriptEvent_Alert) _then) =
      _$BridgeScriptEvent_AlertCopyWithImpl;
  @useResult
  $Res call({String message});
}

/// @nodoc
class _$BridgeScriptEvent_AlertCopyWithImpl<$Res>
    implements $BridgeScriptEvent_AlertCopyWith<$Res> {
  _$BridgeScriptEvent_AlertCopyWithImpl(this._self, this._then);

  final BridgeScriptEvent_Alert _self;
  final $Res Function(BridgeScriptEvent_Alert) _then;

  /// Create a copy of BridgeScriptEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? message = null,
  }) {
    return _then(BridgeScriptEvent_Alert(
      message: null == message
          ? _self.message
          : message // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeScriptEvent_OpenComp extends BridgeScriptEvent {
  const BridgeScriptEvent_OpenComp({required this.comp}) : super._();

  final CompositionReference comp;

  /// Create a copy of BridgeScriptEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeScriptEvent_OpenCompCopyWith<BridgeScriptEvent_OpenComp>
      get copyWith =>
          _$BridgeScriptEvent_OpenCompCopyWithImpl<BridgeScriptEvent_OpenComp>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeScriptEvent_OpenComp &&
            (identical(other.comp, comp) || other.comp == comp));
  }

  @override
  int get hashCode => Object.hash(runtimeType, comp);

  @override
  String toString() {
    return 'BridgeScriptEvent.openComp(comp: $comp)';
  }
}

/// @nodoc
abstract mixin class $BridgeScriptEvent_OpenCompCopyWith<$Res>
    implements $BridgeScriptEventCopyWith<$Res> {
  factory $BridgeScriptEvent_OpenCompCopyWith(BridgeScriptEvent_OpenComp value,
          $Res Function(BridgeScriptEvent_OpenComp) _then) =
      _$BridgeScriptEvent_OpenCompCopyWithImpl;
  @useResult
  $Res call({CompositionReference comp});
}

/// @nodoc
class _$BridgeScriptEvent_OpenCompCopyWithImpl<$Res>
    implements $BridgeScriptEvent_OpenCompCopyWith<$Res> {
  _$BridgeScriptEvent_OpenCompCopyWithImpl(this._self, this._then);

  final BridgeScriptEvent_OpenComp _self;
  final $Res Function(BridgeScriptEvent_OpenComp) _then;

  /// Create a copy of BridgeScriptEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? comp = null,
  }) {
    return _then(BridgeScriptEvent_OpenComp(
      comp: null == comp
          ? _self.comp
          : comp // ignore: cast_nullable_to_non_nullable
              as CompositionReference,
    ));
  }
}

/// @nodoc

class BridgeScriptEvent_Finished extends BridgeScriptEvent {
  const BridgeScriptEvent_Finished(
      {required final List<String> log,
      required this.stopped,
      this.error,
      this.line})
      : _log = log,
        super._();

  /// What it printed.
  final List<String> _log;

  /// What it printed.
  List<String> get log {
    if (_log is EqualUnmodifiableListView) return _log;
    // ignore: implicit_dynamic_type
    return EqualUnmodifiableListView(_log);
  }

  final bool stopped;
  final String? error;
  final int? line;

  /// Create a copy of BridgeScriptEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeScriptEvent_FinishedCopyWith<BridgeScriptEvent_Finished>
      get copyWith =>
          _$BridgeScriptEvent_FinishedCopyWithImpl<BridgeScriptEvent_Finished>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeScriptEvent_Finished &&
            const DeepCollectionEquality().equals(other._log, _log) &&
            (identical(other.stopped, stopped) || other.stopped == stopped) &&
            (identical(other.error, error) || other.error == error) &&
            (identical(other.line, line) || other.line == line));
  }

  @override
  int get hashCode => Object.hash(runtimeType,
      const DeepCollectionEquality().hash(_log), stopped, error, line);

  @override
  String toString() {
    return 'BridgeScriptEvent.finished(log: $log, stopped: $stopped, error: $error, line: $line)';
  }
}

/// @nodoc
abstract mixin class $BridgeScriptEvent_FinishedCopyWith<$Res>
    implements $BridgeScriptEventCopyWith<$Res> {
  factory $BridgeScriptEvent_FinishedCopyWith(BridgeScriptEvent_Finished value,
          $Res Function(BridgeScriptEvent_Finished) _then) =
      _$BridgeScriptEvent_FinishedCopyWithImpl;
  @useResult
  $Res call({List<String> log, bool stopped, String? error, int? line});
}

/// @nodoc
class _$BridgeScriptEvent_FinishedCopyWithImpl<$Res>
    implements $BridgeScriptEvent_FinishedCopyWith<$Res> {
  _$BridgeScriptEvent_FinishedCopyWithImpl(this._self, this._then);

  final BridgeScriptEvent_Finished _self;
  final $Res Function(BridgeScriptEvent_Finished) _then;

  /// Create a copy of BridgeScriptEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? log = null,
    Object? stopped = null,
    Object? error = freezed,
    Object? line = freezed,
  }) {
    return _then(BridgeScriptEvent_Finished(
      log: null == log
          ? _self._log
          : log // ignore: cast_nullable_to_non_nullable
              as List<String>,
      stopped: null == stopped
          ? _self.stopped
          : stopped // ignore: cast_nullable_to_non_nullable
              as bool,
      error: freezed == error
          ? _self.error
          : error // ignore: cast_nullable_to_non_nullable
              as String?,
      line: freezed == line
          ? _self.line
          : line // ignore: cast_nullable_to_non_nullable
              as int?,
    ));
  }
}

// dart format on
