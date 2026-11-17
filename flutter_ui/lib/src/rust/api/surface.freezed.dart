// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'surface.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$BridgeSurfaceEvent {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeSurfaceEvent);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeSurfaceEvent()';
  }
}

/// @nodoc
class $BridgeSurfaceEventCopyWith<$Res> {
  $BridgeSurfaceEventCopyWith(
      BridgeSurfaceEvent _, $Res Function(BridgeSurfaceEvent) __);
}

/// Adds pattern-matching-related methods to [BridgeSurfaceEvent].
extension BridgeSurfaceEventPatterns on BridgeSurfaceEvent {
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
    TResult Function(BridgeSurfaceEvent_Nudge value)? nudge,
    TResult Function(BridgeSurfaceEvent_Reset value)? reset,
    TResult Function(BridgeSurfaceEvent_Action value)? action,
    TResult Function(BridgeSurfaceEvent_Menu value)? menu,
    TResult Function(BridgeSurfaceEvent_Jog value)? jog,
    TResult Function(BridgeSurfaceEvent_Shuttle value)? shuttle,
    TResult Function(BridgeSurfaceEvent_Tangent value)? tangent,
    TResult Function(BridgeSurfaceEvent_Ports value)? ports,
    TResult Function(BridgeSurfaceEvent_Learned value)? learned,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSurfaceEvent_Nudge() when nudge != null:
        return nudge(_that);
      case BridgeSurfaceEvent_Reset() when reset != null:
        return reset(_that);
      case BridgeSurfaceEvent_Action() when action != null:
        return action(_that);
      case BridgeSurfaceEvent_Menu() when menu != null:
        return menu(_that);
      case BridgeSurfaceEvent_Jog() when jog != null:
        return jog(_that);
      case BridgeSurfaceEvent_Shuttle() when shuttle != null:
        return shuttle(_that);
      case BridgeSurfaceEvent_Tangent() when tangent != null:
        return tangent(_that);
      case BridgeSurfaceEvent_Ports() when ports != null:
        return ports(_that);
      case BridgeSurfaceEvent_Learned() when learned != null:
        return learned(_that);
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
    required TResult Function(BridgeSurfaceEvent_Nudge value) nudge,
    required TResult Function(BridgeSurfaceEvent_Reset value) reset,
    required TResult Function(BridgeSurfaceEvent_Action value) action,
    required TResult Function(BridgeSurfaceEvent_Menu value) menu,
    required TResult Function(BridgeSurfaceEvent_Jog value) jog,
    required TResult Function(BridgeSurfaceEvent_Shuttle value) shuttle,
    required TResult Function(BridgeSurfaceEvent_Tangent value) tangent,
    required TResult Function(BridgeSurfaceEvent_Ports value) ports,
    required TResult Function(BridgeSurfaceEvent_Learned value) learned,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSurfaceEvent_Nudge():
        return nudge(_that);
      case BridgeSurfaceEvent_Reset():
        return reset(_that);
      case BridgeSurfaceEvent_Action():
        return action(_that);
      case BridgeSurfaceEvent_Menu():
        return menu(_that);
      case BridgeSurfaceEvent_Jog():
        return jog(_that);
      case BridgeSurfaceEvent_Shuttle():
        return shuttle(_that);
      case BridgeSurfaceEvent_Tangent():
        return tangent(_that);
      case BridgeSurfaceEvent_Ports():
        return ports(_that);
      case BridgeSurfaceEvent_Learned():
        return learned(_that);
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
    TResult? Function(BridgeSurfaceEvent_Nudge value)? nudge,
    TResult? Function(BridgeSurfaceEvent_Reset value)? reset,
    TResult? Function(BridgeSurfaceEvent_Action value)? action,
    TResult? Function(BridgeSurfaceEvent_Menu value)? menu,
    TResult? Function(BridgeSurfaceEvent_Jog value)? jog,
    TResult? Function(BridgeSurfaceEvent_Shuttle value)? shuttle,
    TResult? Function(BridgeSurfaceEvent_Tangent value)? tangent,
    TResult? Function(BridgeSurfaceEvent_Ports value)? ports,
    TResult? Function(BridgeSurfaceEvent_Learned value)? learned,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSurfaceEvent_Nudge() when nudge != null:
        return nudge(_that);
      case BridgeSurfaceEvent_Reset() when reset != null:
        return reset(_that);
      case BridgeSurfaceEvent_Action() when action != null:
        return action(_that);
      case BridgeSurfaceEvent_Menu() when menu != null:
        return menu(_that);
      case BridgeSurfaceEvent_Jog() when jog != null:
        return jog(_that);
      case BridgeSurfaceEvent_Shuttle() when shuttle != null:
        return shuttle(_that);
      case BridgeSurfaceEvent_Tangent() when tangent != null:
        return tangent(_that);
      case BridgeSurfaceEvent_Ports() when ports != null:
        return ports(_that);
      case BridgeSurfaceEvent_Learned() when learned != null:
        return learned(_that);
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
    TResult Function(String param, BridgeSurfaceAxis? axis, double by)? nudge,
    TResult Function(String param, BridgeSurfaceAxis? axis)? reset,
    TResult Function(String id, bool down)? action,
    TResult Function(String id, int step)? menu,
    TResult Function(int frames)? jog,
    TResult Function(int step)? shuttle,
    TResult Function(BridgeTangentStatus status)? tangent,
    TResult Function(List<String> ports)? ports,
    TResult Function(BridgeMidiControl control, BridgeMidiEncoding encoding)?
        learned,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSurfaceEvent_Nudge() when nudge != null:
        return nudge(_that.param, _that.axis, _that.by);
      case BridgeSurfaceEvent_Reset() when reset != null:
        return reset(_that.param, _that.axis);
      case BridgeSurfaceEvent_Action() when action != null:
        return action(_that.id, _that.down);
      case BridgeSurfaceEvent_Menu() when menu != null:
        return menu(_that.id, _that.step);
      case BridgeSurfaceEvent_Jog() when jog != null:
        return jog(_that.frames);
      case BridgeSurfaceEvent_Shuttle() when shuttle != null:
        return shuttle(_that.step);
      case BridgeSurfaceEvent_Tangent() when tangent != null:
        return tangent(_that.status);
      case BridgeSurfaceEvent_Ports() when ports != null:
        return ports(_that.ports);
      case BridgeSurfaceEvent_Learned() when learned != null:
        return learned(_that.control, _that.encoding);
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
    required TResult Function(String param, BridgeSurfaceAxis? axis, double by)
        nudge,
    required TResult Function(String param, BridgeSurfaceAxis? axis) reset,
    required TResult Function(String id, bool down) action,
    required TResult Function(String id, int step) menu,
    required TResult Function(int frames) jog,
    required TResult Function(int step) shuttle,
    required TResult Function(BridgeTangentStatus status) tangent,
    required TResult Function(List<String> ports) ports,
    required TResult Function(
            BridgeMidiControl control, BridgeMidiEncoding encoding)
        learned,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSurfaceEvent_Nudge():
        return nudge(_that.param, _that.axis, _that.by);
      case BridgeSurfaceEvent_Reset():
        return reset(_that.param, _that.axis);
      case BridgeSurfaceEvent_Action():
        return action(_that.id, _that.down);
      case BridgeSurfaceEvent_Menu():
        return menu(_that.id, _that.step);
      case BridgeSurfaceEvent_Jog():
        return jog(_that.frames);
      case BridgeSurfaceEvent_Shuttle():
        return shuttle(_that.step);
      case BridgeSurfaceEvent_Tangent():
        return tangent(_that.status);
      case BridgeSurfaceEvent_Ports():
        return ports(_that.ports);
      case BridgeSurfaceEvent_Learned():
        return learned(_that.control, _that.encoding);
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
    TResult? Function(String param, BridgeSurfaceAxis? axis, double by)? nudge,
    TResult? Function(String param, BridgeSurfaceAxis? axis)? reset,
    TResult? Function(String id, bool down)? action,
    TResult? Function(String id, int step)? menu,
    TResult? Function(int frames)? jog,
    TResult? Function(int step)? shuttle,
    TResult? Function(BridgeTangentStatus status)? tangent,
    TResult? Function(List<String> ports)? ports,
    TResult? Function(BridgeMidiControl control, BridgeMidiEncoding encoding)?
        learned,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSurfaceEvent_Nudge() when nudge != null:
        return nudge(_that.param, _that.axis, _that.by);
      case BridgeSurfaceEvent_Reset() when reset != null:
        return reset(_that.param, _that.axis);
      case BridgeSurfaceEvent_Action() when action != null:
        return action(_that.id, _that.down);
      case BridgeSurfaceEvent_Menu() when menu != null:
        return menu(_that.id, _that.step);
      case BridgeSurfaceEvent_Jog() when jog != null:
        return jog(_that.frames);
      case BridgeSurfaceEvent_Shuttle() when shuttle != null:
        return shuttle(_that.step);
      case BridgeSurfaceEvent_Tangent() when tangent != null:
        return tangent(_that.status);
      case BridgeSurfaceEvent_Ports() when ports != null:
        return ports(_that.ports);
      case BridgeSurfaceEvent_Learned() when learned != null:
        return learned(_that.control, _that.encoding);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeSurfaceEvent_Nudge extends BridgeSurfaceEvent {
  const BridgeSurfaceEvent_Nudge(
      {required this.param, this.axis, required this.by})
      : super._();

  final String param;
  final BridgeSurfaceAxis? axis;
  final double by;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeSurfaceEvent_NudgeCopyWith<BridgeSurfaceEvent_Nudge> get copyWith =>
      _$BridgeSurfaceEvent_NudgeCopyWithImpl<BridgeSurfaceEvent_Nudge>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeSurfaceEvent_Nudge &&
            (identical(other.param, param) || other.param == param) &&
            (identical(other.axis, axis) || other.axis == axis) &&
            (identical(other.by, by) || other.by == by));
  }

  @override
  int get hashCode => Object.hash(runtimeType, param, axis, by);

  @override
  String toString() {
    return 'BridgeSurfaceEvent.nudge(param: $param, axis: $axis, by: $by)';
  }
}

/// @nodoc
abstract mixin class $BridgeSurfaceEvent_NudgeCopyWith<$Res>
    implements $BridgeSurfaceEventCopyWith<$Res> {
  factory $BridgeSurfaceEvent_NudgeCopyWith(BridgeSurfaceEvent_Nudge value,
          $Res Function(BridgeSurfaceEvent_Nudge) _then) =
      _$BridgeSurfaceEvent_NudgeCopyWithImpl;
  @useResult
  $Res call({String param, BridgeSurfaceAxis? axis, double by});
}

/// @nodoc
class _$BridgeSurfaceEvent_NudgeCopyWithImpl<$Res>
    implements $BridgeSurfaceEvent_NudgeCopyWith<$Res> {
  _$BridgeSurfaceEvent_NudgeCopyWithImpl(this._self, this._then);

  final BridgeSurfaceEvent_Nudge _self;
  final $Res Function(BridgeSurfaceEvent_Nudge) _then;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? param = null,
    Object? axis = freezed,
    Object? by = null,
  }) {
    return _then(BridgeSurfaceEvent_Nudge(
      param: null == param
          ? _self.param
          : param // ignore: cast_nullable_to_non_nullable
              as String,
      axis: freezed == axis
          ? _self.axis
          : axis // ignore: cast_nullable_to_non_nullable
              as BridgeSurfaceAxis?,
      by: null == by
          ? _self.by
          : by // ignore: cast_nullable_to_non_nullable
              as double,
    ));
  }
}

/// @nodoc

class BridgeSurfaceEvent_Reset extends BridgeSurfaceEvent {
  const BridgeSurfaceEvent_Reset({required this.param, this.axis}) : super._();

  final String param;
  final BridgeSurfaceAxis? axis;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeSurfaceEvent_ResetCopyWith<BridgeSurfaceEvent_Reset> get copyWith =>
      _$BridgeSurfaceEvent_ResetCopyWithImpl<BridgeSurfaceEvent_Reset>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeSurfaceEvent_Reset &&
            (identical(other.param, param) || other.param == param) &&
            (identical(other.axis, axis) || other.axis == axis));
  }

  @override
  int get hashCode => Object.hash(runtimeType, param, axis);

  @override
  String toString() {
    return 'BridgeSurfaceEvent.reset(param: $param, axis: $axis)';
  }
}

/// @nodoc
abstract mixin class $BridgeSurfaceEvent_ResetCopyWith<$Res>
    implements $BridgeSurfaceEventCopyWith<$Res> {
  factory $BridgeSurfaceEvent_ResetCopyWith(BridgeSurfaceEvent_Reset value,
          $Res Function(BridgeSurfaceEvent_Reset) _then) =
      _$BridgeSurfaceEvent_ResetCopyWithImpl;
  @useResult
  $Res call({String param, BridgeSurfaceAxis? axis});
}

/// @nodoc
class _$BridgeSurfaceEvent_ResetCopyWithImpl<$Res>
    implements $BridgeSurfaceEvent_ResetCopyWith<$Res> {
  _$BridgeSurfaceEvent_ResetCopyWithImpl(this._self, this._then);

  final BridgeSurfaceEvent_Reset _self;
  final $Res Function(BridgeSurfaceEvent_Reset) _then;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? param = null,
    Object? axis = freezed,
  }) {
    return _then(BridgeSurfaceEvent_Reset(
      param: null == param
          ? _self.param
          : param // ignore: cast_nullable_to_non_nullable
              as String,
      axis: freezed == axis
          ? _self.axis
          : axis // ignore: cast_nullable_to_non_nullable
              as BridgeSurfaceAxis?,
    ));
  }
}

/// @nodoc

class BridgeSurfaceEvent_Action extends BridgeSurfaceEvent {
  const BridgeSurfaceEvent_Action({required this.id, required this.down})
      : super._();

  final String id;
  final bool down;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeSurfaceEvent_ActionCopyWith<BridgeSurfaceEvent_Action> get copyWith =>
      _$BridgeSurfaceEvent_ActionCopyWithImpl<BridgeSurfaceEvent_Action>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeSurfaceEvent_Action &&
            (identical(other.id, id) || other.id == id) &&
            (identical(other.down, down) || other.down == down));
  }

  @override
  int get hashCode => Object.hash(runtimeType, id, down);

  @override
  String toString() {
    return 'BridgeSurfaceEvent.action(id: $id, down: $down)';
  }
}

/// @nodoc
abstract mixin class $BridgeSurfaceEvent_ActionCopyWith<$Res>
    implements $BridgeSurfaceEventCopyWith<$Res> {
  factory $BridgeSurfaceEvent_ActionCopyWith(BridgeSurfaceEvent_Action value,
          $Res Function(BridgeSurfaceEvent_Action) _then) =
      _$BridgeSurfaceEvent_ActionCopyWithImpl;
  @useResult
  $Res call({String id, bool down});
}

/// @nodoc
class _$BridgeSurfaceEvent_ActionCopyWithImpl<$Res>
    implements $BridgeSurfaceEvent_ActionCopyWith<$Res> {
  _$BridgeSurfaceEvent_ActionCopyWithImpl(this._self, this._then);

  final BridgeSurfaceEvent_Action _self;
  final $Res Function(BridgeSurfaceEvent_Action) _then;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? id = null,
    Object? down = null,
  }) {
    return _then(BridgeSurfaceEvent_Action(
      id: null == id
          ? _self.id
          : id // ignore: cast_nullable_to_non_nullable
              as String,
      down: null == down
          ? _self.down
          : down // ignore: cast_nullable_to_non_nullable
              as bool,
    ));
  }
}

/// @nodoc

class BridgeSurfaceEvent_Menu extends BridgeSurfaceEvent {
  const BridgeSurfaceEvent_Menu({required this.id, required this.step})
      : super._();

  final String id;
  final int step;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeSurfaceEvent_MenuCopyWith<BridgeSurfaceEvent_Menu> get copyWith =>
      _$BridgeSurfaceEvent_MenuCopyWithImpl<BridgeSurfaceEvent_Menu>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeSurfaceEvent_Menu &&
            (identical(other.id, id) || other.id == id) &&
            (identical(other.step, step) || other.step == step));
  }

  @override
  int get hashCode => Object.hash(runtimeType, id, step);

  @override
  String toString() {
    return 'BridgeSurfaceEvent.menu(id: $id, step: $step)';
  }
}

/// @nodoc
abstract mixin class $BridgeSurfaceEvent_MenuCopyWith<$Res>
    implements $BridgeSurfaceEventCopyWith<$Res> {
  factory $BridgeSurfaceEvent_MenuCopyWith(BridgeSurfaceEvent_Menu value,
          $Res Function(BridgeSurfaceEvent_Menu) _then) =
      _$BridgeSurfaceEvent_MenuCopyWithImpl;
  @useResult
  $Res call({String id, int step});
}

/// @nodoc
class _$BridgeSurfaceEvent_MenuCopyWithImpl<$Res>
    implements $BridgeSurfaceEvent_MenuCopyWith<$Res> {
  _$BridgeSurfaceEvent_MenuCopyWithImpl(this._self, this._then);

  final BridgeSurfaceEvent_Menu _self;
  final $Res Function(BridgeSurfaceEvent_Menu) _then;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? id = null,
    Object? step = null,
  }) {
    return _then(BridgeSurfaceEvent_Menu(
      id: null == id
          ? _self.id
          : id // ignore: cast_nullable_to_non_nullable
              as String,
      step: null == step
          ? _self.step
          : step // ignore: cast_nullable_to_non_nullable
              as int,
    ));
  }
}

/// @nodoc

class BridgeSurfaceEvent_Jog extends BridgeSurfaceEvent {
  const BridgeSurfaceEvent_Jog({required this.frames}) : super._();

  final int frames;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeSurfaceEvent_JogCopyWith<BridgeSurfaceEvent_Jog> get copyWith =>
      _$BridgeSurfaceEvent_JogCopyWithImpl<BridgeSurfaceEvent_Jog>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeSurfaceEvent_Jog &&
            (identical(other.frames, frames) || other.frames == frames));
  }

  @override
  int get hashCode => Object.hash(runtimeType, frames);

  @override
  String toString() {
    return 'BridgeSurfaceEvent.jog(frames: $frames)';
  }
}

/// @nodoc
abstract mixin class $BridgeSurfaceEvent_JogCopyWith<$Res>
    implements $BridgeSurfaceEventCopyWith<$Res> {
  factory $BridgeSurfaceEvent_JogCopyWith(BridgeSurfaceEvent_Jog value,
          $Res Function(BridgeSurfaceEvent_Jog) _then) =
      _$BridgeSurfaceEvent_JogCopyWithImpl;
  @useResult
  $Res call({int frames});
}

/// @nodoc
class _$BridgeSurfaceEvent_JogCopyWithImpl<$Res>
    implements $BridgeSurfaceEvent_JogCopyWith<$Res> {
  _$BridgeSurfaceEvent_JogCopyWithImpl(this._self, this._then);

  final BridgeSurfaceEvent_Jog _self;
  final $Res Function(BridgeSurfaceEvent_Jog) _then;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? frames = null,
  }) {
    return _then(BridgeSurfaceEvent_Jog(
      frames: null == frames
          ? _self.frames
          : frames // ignore: cast_nullable_to_non_nullable
              as int,
    ));
  }
}

/// @nodoc

class BridgeSurfaceEvent_Shuttle extends BridgeSurfaceEvent {
  const BridgeSurfaceEvent_Shuttle({required this.step}) : super._();

  final int step;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeSurfaceEvent_ShuttleCopyWith<BridgeSurfaceEvent_Shuttle>
      get copyWith =>
          _$BridgeSurfaceEvent_ShuttleCopyWithImpl<BridgeSurfaceEvent_Shuttle>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeSurfaceEvent_Shuttle &&
            (identical(other.step, step) || other.step == step));
  }

  @override
  int get hashCode => Object.hash(runtimeType, step);

  @override
  String toString() {
    return 'BridgeSurfaceEvent.shuttle(step: $step)';
  }
}

/// @nodoc
abstract mixin class $BridgeSurfaceEvent_ShuttleCopyWith<$Res>
    implements $BridgeSurfaceEventCopyWith<$Res> {
  factory $BridgeSurfaceEvent_ShuttleCopyWith(BridgeSurfaceEvent_Shuttle value,
          $Res Function(BridgeSurfaceEvent_Shuttle) _then) =
      _$BridgeSurfaceEvent_ShuttleCopyWithImpl;
  @useResult
  $Res call({int step});
}

/// @nodoc
class _$BridgeSurfaceEvent_ShuttleCopyWithImpl<$Res>
    implements $BridgeSurfaceEvent_ShuttleCopyWith<$Res> {
  _$BridgeSurfaceEvent_ShuttleCopyWithImpl(this._self, this._then);

  final BridgeSurfaceEvent_Shuttle _self;
  final $Res Function(BridgeSurfaceEvent_Shuttle) _then;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? step = null,
  }) {
    return _then(BridgeSurfaceEvent_Shuttle(
      step: null == step
          ? _self.step
          : step // ignore: cast_nullable_to_non_nullable
              as int,
    ));
  }
}

/// @nodoc

class BridgeSurfaceEvent_Tangent extends BridgeSurfaceEvent {
  const BridgeSurfaceEvent_Tangent({required this.status}) : super._();

  final BridgeTangentStatus status;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeSurfaceEvent_TangentCopyWith<BridgeSurfaceEvent_Tangent>
      get copyWith =>
          _$BridgeSurfaceEvent_TangentCopyWithImpl<BridgeSurfaceEvent_Tangent>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeSurfaceEvent_Tangent &&
            (identical(other.status, status) || other.status == status));
  }

  @override
  int get hashCode => Object.hash(runtimeType, status);

  @override
  String toString() {
    return 'BridgeSurfaceEvent.tangent(status: $status)';
  }
}

/// @nodoc
abstract mixin class $BridgeSurfaceEvent_TangentCopyWith<$Res>
    implements $BridgeSurfaceEventCopyWith<$Res> {
  factory $BridgeSurfaceEvent_TangentCopyWith(BridgeSurfaceEvent_Tangent value,
          $Res Function(BridgeSurfaceEvent_Tangent) _then) =
      _$BridgeSurfaceEvent_TangentCopyWithImpl;
  @useResult
  $Res call({BridgeTangentStatus status});

  $BridgeTangentStatusCopyWith<$Res> get status;
}

/// @nodoc
class _$BridgeSurfaceEvent_TangentCopyWithImpl<$Res>
    implements $BridgeSurfaceEvent_TangentCopyWith<$Res> {
  _$BridgeSurfaceEvent_TangentCopyWithImpl(this._self, this._then);

  final BridgeSurfaceEvent_Tangent _self;
  final $Res Function(BridgeSurfaceEvent_Tangent) _then;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? status = null,
  }) {
    return _then(BridgeSurfaceEvent_Tangent(
      status: null == status
          ? _self.status
          : status // ignore: cast_nullable_to_non_nullable
              as BridgeTangentStatus,
    ));
  }

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @override
  @pragma('vm:prefer-inline')
  $BridgeTangentStatusCopyWith<$Res> get status {
    return $BridgeTangentStatusCopyWith<$Res>(_self.status, (value) {
      return _then(_self.copyWith(status: value));
    });
  }
}

/// @nodoc

class BridgeSurfaceEvent_Ports extends BridgeSurfaceEvent {
  const BridgeSurfaceEvent_Ports({required final List<String> ports})
      : _ports = ports,
        super._();

  final List<String> _ports;
  List<String> get ports {
    if (_ports is EqualUnmodifiableListView) return _ports;
    // ignore: implicit_dynamic_type
    return EqualUnmodifiableListView(_ports);
  }

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeSurfaceEvent_PortsCopyWith<BridgeSurfaceEvent_Ports> get copyWith =>
      _$BridgeSurfaceEvent_PortsCopyWithImpl<BridgeSurfaceEvent_Ports>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeSurfaceEvent_Ports &&
            const DeepCollectionEquality().equals(other._ports, _ports));
  }

  @override
  int get hashCode =>
      Object.hash(runtimeType, const DeepCollectionEquality().hash(_ports));

  @override
  String toString() {
    return 'BridgeSurfaceEvent.ports(ports: $ports)';
  }
}

/// @nodoc
abstract mixin class $BridgeSurfaceEvent_PortsCopyWith<$Res>
    implements $BridgeSurfaceEventCopyWith<$Res> {
  factory $BridgeSurfaceEvent_PortsCopyWith(BridgeSurfaceEvent_Ports value,
          $Res Function(BridgeSurfaceEvent_Ports) _then) =
      _$BridgeSurfaceEvent_PortsCopyWithImpl;
  @useResult
  $Res call({List<String> ports});
}

/// @nodoc
class _$BridgeSurfaceEvent_PortsCopyWithImpl<$Res>
    implements $BridgeSurfaceEvent_PortsCopyWith<$Res> {
  _$BridgeSurfaceEvent_PortsCopyWithImpl(this._self, this._then);

  final BridgeSurfaceEvent_Ports _self;
  final $Res Function(BridgeSurfaceEvent_Ports) _then;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? ports = null,
  }) {
    return _then(BridgeSurfaceEvent_Ports(
      ports: null == ports
          ? _self._ports
          : ports // ignore: cast_nullable_to_non_nullable
              as List<String>,
    ));
  }
}

/// @nodoc

class BridgeSurfaceEvent_Learned extends BridgeSurfaceEvent {
  const BridgeSurfaceEvent_Learned(
      {required this.control, required this.encoding})
      : super._();

  final BridgeMidiControl control;
  final BridgeMidiEncoding encoding;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeSurfaceEvent_LearnedCopyWith<BridgeSurfaceEvent_Learned>
      get copyWith =>
          _$BridgeSurfaceEvent_LearnedCopyWithImpl<BridgeSurfaceEvent_Learned>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeSurfaceEvent_Learned &&
            (identical(other.control, control) || other.control == control) &&
            (identical(other.encoding, encoding) ||
                other.encoding == encoding));
  }

  @override
  int get hashCode => Object.hash(runtimeType, control, encoding);

  @override
  String toString() {
    return 'BridgeSurfaceEvent.learned(control: $control, encoding: $encoding)';
  }
}

/// @nodoc
abstract mixin class $BridgeSurfaceEvent_LearnedCopyWith<$Res>
    implements $BridgeSurfaceEventCopyWith<$Res> {
  factory $BridgeSurfaceEvent_LearnedCopyWith(BridgeSurfaceEvent_Learned value,
          $Res Function(BridgeSurfaceEvent_Learned) _then) =
      _$BridgeSurfaceEvent_LearnedCopyWithImpl;
  @useResult
  $Res call({BridgeMidiControl control, BridgeMidiEncoding encoding});
}

/// @nodoc
class _$BridgeSurfaceEvent_LearnedCopyWithImpl<$Res>
    implements $BridgeSurfaceEvent_LearnedCopyWith<$Res> {
  _$BridgeSurfaceEvent_LearnedCopyWithImpl(this._self, this._then);

  final BridgeSurfaceEvent_Learned _self;
  final $Res Function(BridgeSurfaceEvent_Learned) _then;

  /// Create a copy of BridgeSurfaceEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? control = null,
    Object? encoding = null,
  }) {
    return _then(BridgeSurfaceEvent_Learned(
      control: null == control
          ? _self.control
          : control // ignore: cast_nullable_to_non_nullable
              as BridgeMidiControl,
      encoding: null == encoding
          ? _self.encoding
          : encoding // ignore: cast_nullable_to_non_nullable
              as BridgeMidiEncoding,
    ));
  }
}

/// @nodoc
mixin _$BridgeSurfaceTarget {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeSurfaceTarget);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeSurfaceTarget()';
  }
}

/// @nodoc
class $BridgeSurfaceTargetCopyWith<$Res> {
  $BridgeSurfaceTargetCopyWith(
      BridgeSurfaceTarget _, $Res Function(BridgeSurfaceTarget) __);
}

/// Adds pattern-matching-related methods to [BridgeSurfaceTarget].
extension BridgeSurfaceTargetPatterns on BridgeSurfaceTarget {
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
    TResult Function(BridgeSurfaceTarget_Param value)? param,
    TResult Function(BridgeSurfaceTarget_Action value)? action,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSurfaceTarget_Param() when param != null:
        return param(_that);
      case BridgeSurfaceTarget_Action() when action != null:
        return action(_that);
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
    required TResult Function(BridgeSurfaceTarget_Param value) param,
    required TResult Function(BridgeSurfaceTarget_Action value) action,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSurfaceTarget_Param():
        return param(_that);
      case BridgeSurfaceTarget_Action():
        return action(_that);
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
    TResult? Function(BridgeSurfaceTarget_Param value)? param,
    TResult? Function(BridgeSurfaceTarget_Action value)? action,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSurfaceTarget_Param() when param != null:
        return param(_that);
      case BridgeSurfaceTarget_Action() when action != null:
        return action(_that);
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
    TResult Function(String param, BridgeSurfaceAxis? axis)? param,
    TResult Function(String id)? action,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSurfaceTarget_Param() when param != null:
        return param(_that.param, _that.axis);
      case BridgeSurfaceTarget_Action() when action != null:
        return action(_that.id);
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
    required TResult Function(String param, BridgeSurfaceAxis? axis) param,
    required TResult Function(String id) action,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSurfaceTarget_Param():
        return param(_that.param, _that.axis);
      case BridgeSurfaceTarget_Action():
        return action(_that.id);
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
    TResult? Function(String param, BridgeSurfaceAxis? axis)? param,
    TResult? Function(String id)? action,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeSurfaceTarget_Param() when param != null:
        return param(_that.param, _that.axis);
      case BridgeSurfaceTarget_Action() when action != null:
        return action(_that.id);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeSurfaceTarget_Param extends BridgeSurfaceTarget {
  const BridgeSurfaceTarget_Param({required this.param, this.axis}) : super._();

  final String param;
  final BridgeSurfaceAxis? axis;

  /// Create a copy of BridgeSurfaceTarget
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeSurfaceTarget_ParamCopyWith<BridgeSurfaceTarget_Param> get copyWith =>
      _$BridgeSurfaceTarget_ParamCopyWithImpl<BridgeSurfaceTarget_Param>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeSurfaceTarget_Param &&
            (identical(other.param, param) || other.param == param) &&
            (identical(other.axis, axis) || other.axis == axis));
  }

  @override
  int get hashCode => Object.hash(runtimeType, param, axis);

  @override
  String toString() {
    return 'BridgeSurfaceTarget.param(param: $param, axis: $axis)';
  }
}

/// @nodoc
abstract mixin class $BridgeSurfaceTarget_ParamCopyWith<$Res>
    implements $BridgeSurfaceTargetCopyWith<$Res> {
  factory $BridgeSurfaceTarget_ParamCopyWith(BridgeSurfaceTarget_Param value,
          $Res Function(BridgeSurfaceTarget_Param) _then) =
      _$BridgeSurfaceTarget_ParamCopyWithImpl;
  @useResult
  $Res call({String param, BridgeSurfaceAxis? axis});
}

/// @nodoc
class _$BridgeSurfaceTarget_ParamCopyWithImpl<$Res>
    implements $BridgeSurfaceTarget_ParamCopyWith<$Res> {
  _$BridgeSurfaceTarget_ParamCopyWithImpl(this._self, this._then);

  final BridgeSurfaceTarget_Param _self;
  final $Res Function(BridgeSurfaceTarget_Param) _then;

  /// Create a copy of BridgeSurfaceTarget
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? param = null,
    Object? axis = freezed,
  }) {
    return _then(BridgeSurfaceTarget_Param(
      param: null == param
          ? _self.param
          : param // ignore: cast_nullable_to_non_nullable
              as String,
      axis: freezed == axis
          ? _self.axis
          : axis // ignore: cast_nullable_to_non_nullable
              as BridgeSurfaceAxis?,
    ));
  }
}

/// @nodoc

class BridgeSurfaceTarget_Action extends BridgeSurfaceTarget {
  const BridgeSurfaceTarget_Action({required this.id}) : super._();

  final String id;

  /// Create a copy of BridgeSurfaceTarget
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeSurfaceTarget_ActionCopyWith<BridgeSurfaceTarget_Action>
      get copyWith =>
          _$BridgeSurfaceTarget_ActionCopyWithImpl<BridgeSurfaceTarget_Action>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeSurfaceTarget_Action &&
            (identical(other.id, id) || other.id == id));
  }

  @override
  int get hashCode => Object.hash(runtimeType, id);

  @override
  String toString() {
    return 'BridgeSurfaceTarget.action(id: $id)';
  }
}

/// @nodoc
abstract mixin class $BridgeSurfaceTarget_ActionCopyWith<$Res>
    implements $BridgeSurfaceTargetCopyWith<$Res> {
  factory $BridgeSurfaceTarget_ActionCopyWith(BridgeSurfaceTarget_Action value,
          $Res Function(BridgeSurfaceTarget_Action) _then) =
      _$BridgeSurfaceTarget_ActionCopyWithImpl;
  @useResult
  $Res call({String id});
}

/// @nodoc
class _$BridgeSurfaceTarget_ActionCopyWithImpl<$Res>
    implements $BridgeSurfaceTarget_ActionCopyWith<$Res> {
  _$BridgeSurfaceTarget_ActionCopyWithImpl(this._self, this._then);

  final BridgeSurfaceTarget_Action _self;
  final $Res Function(BridgeSurfaceTarget_Action) _then;

  /// Create a copy of BridgeSurfaceTarget
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? id = null,
  }) {
    return _then(BridgeSurfaceTarget_Action(
      id: null == id
          ? _self.id
          : id // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc
mixin _$BridgeTangentStatus {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeTangentStatus);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeTangentStatus()';
  }
}

/// @nodoc
class $BridgeTangentStatusCopyWith<$Res> {
  $BridgeTangentStatusCopyWith(
      BridgeTangentStatus _, $Res Function(BridgeTangentStatus) __);
}

/// Adds pattern-matching-related methods to [BridgeTangentStatus].
extension BridgeTangentStatusPatterns on BridgeTangentStatus {
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
    TResult Function(BridgeTangentStatus_Off value)? off,
    TResult Function(BridgeTangentStatus_Waiting value)? waiting,
    TResult Function(BridgeTangentStatus_Connected value)? connected,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeTangentStatus_Off() when off != null:
        return off(_that);
      case BridgeTangentStatus_Waiting() when waiting != null:
        return waiting(_that);
      case BridgeTangentStatus_Connected() when connected != null:
        return connected(_that);
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
    required TResult Function(BridgeTangentStatus_Off value) off,
    required TResult Function(BridgeTangentStatus_Waiting value) waiting,
    required TResult Function(BridgeTangentStatus_Connected value) connected,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeTangentStatus_Off():
        return off(_that);
      case BridgeTangentStatus_Waiting():
        return waiting(_that);
      case BridgeTangentStatus_Connected():
        return connected(_that);
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
    TResult? Function(BridgeTangentStatus_Off value)? off,
    TResult? Function(BridgeTangentStatus_Waiting value)? waiting,
    TResult? Function(BridgeTangentStatus_Connected value)? connected,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeTangentStatus_Off() when off != null:
        return off(_that);
      case BridgeTangentStatus_Waiting() when waiting != null:
        return waiting(_that);
      case BridgeTangentStatus_Connected() when connected != null:
        return connected(_that);
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
    TResult Function()? off,
    TResult Function()? waiting,
    TResult Function(List<String> panels)? connected,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeTangentStatus_Off() when off != null:
        return off();
      case BridgeTangentStatus_Waiting() when waiting != null:
        return waiting();
      case BridgeTangentStatus_Connected() when connected != null:
        return connected(_that.panels);
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
    required TResult Function() off,
    required TResult Function() waiting,
    required TResult Function(List<String> panels) connected,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeTangentStatus_Off():
        return off();
      case BridgeTangentStatus_Waiting():
        return waiting();
      case BridgeTangentStatus_Connected():
        return connected(_that.panels);
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
    TResult? Function()? off,
    TResult? Function()? waiting,
    TResult? Function(List<String> panels)? connected,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeTangentStatus_Off() when off != null:
        return off();
      case BridgeTangentStatus_Waiting() when waiting != null:
        return waiting();
      case BridgeTangentStatus_Connected() when connected != null:
        return connected(_that.panels);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeTangentStatus_Off extends BridgeTangentStatus {
  const BridgeTangentStatus_Off() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeTangentStatus_Off);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeTangentStatus.off()';
  }
}

/// @nodoc

class BridgeTangentStatus_Waiting extends BridgeTangentStatus {
  const BridgeTangentStatus_Waiting() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeTangentStatus_Waiting);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeTangentStatus.waiting()';
  }
}

/// @nodoc

class BridgeTangentStatus_Connected extends BridgeTangentStatus {
  const BridgeTangentStatus_Connected({required final List<String> panels})
      : _panels = panels,
        super._();

  final List<String> _panels;
  List<String> get panels {
    if (_panels is EqualUnmodifiableListView) return _panels;
    // ignore: implicit_dynamic_type
    return EqualUnmodifiableListView(_panels);
  }

  /// Create a copy of BridgeTangentStatus
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeTangentStatus_ConnectedCopyWith<BridgeTangentStatus_Connected>
      get copyWith => _$BridgeTangentStatus_ConnectedCopyWithImpl<
          BridgeTangentStatus_Connected>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeTangentStatus_Connected &&
            const DeepCollectionEquality().equals(other._panels, _panels));
  }

  @override
  int get hashCode =>
      Object.hash(runtimeType, const DeepCollectionEquality().hash(_panels));

  @override
  String toString() {
    return 'BridgeTangentStatus.connected(panels: $panels)';
  }
}

/// @nodoc
abstract mixin class $BridgeTangentStatus_ConnectedCopyWith<$Res>
    implements $BridgeTangentStatusCopyWith<$Res> {
  factory $BridgeTangentStatus_ConnectedCopyWith(
          BridgeTangentStatus_Connected value,
          $Res Function(BridgeTangentStatus_Connected) _then) =
      _$BridgeTangentStatus_ConnectedCopyWithImpl;
  @useResult
  $Res call({List<String> panels});
}

/// @nodoc
class _$BridgeTangentStatus_ConnectedCopyWithImpl<$Res>
    implements $BridgeTangentStatus_ConnectedCopyWith<$Res> {
  _$BridgeTangentStatus_ConnectedCopyWithImpl(this._self, this._then);

  final BridgeTangentStatus_Connected _self;
  final $Res Function(BridgeTangentStatus_Connected) _then;

  /// Create a copy of BridgeTangentStatus
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? panels = null,
  }) {
    return _then(BridgeTangentStatus_Connected(
      panels: null == panels
          ? _self._panels
          : panels // ignore: cast_nullable_to_non_nullable
              as List<String>,
    ));
  }
}

// dart format on
