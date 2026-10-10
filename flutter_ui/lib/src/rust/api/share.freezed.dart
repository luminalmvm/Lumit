// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'share.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$BridgeJoinOutcome {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeJoinOutcome);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeJoinOutcome()';
  }
}

/// @nodoc
class $BridgeJoinOutcomeCopyWith<$Res> {
  $BridgeJoinOutcomeCopyWith(
      BridgeJoinOutcome _, $Res Function(BridgeJoinOutcome) __);
}

/// Adds pattern-matching-related methods to [BridgeJoinOutcome].
extension BridgeJoinOutcomePatterns on BridgeJoinOutcome {
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
    TResult Function(BridgeJoinOutcome_Joined value)? joined,
    TResult Function(BridgeJoinOutcome_BadInvite value)? badInvite,
    TResult Function(BridgeJoinOutcome_PasswordNeeded value)? passwordNeeded,
    TResult Function(BridgeJoinOutcome_Unreachable value)? unreachable,
    TResult Function(BridgeJoinOutcome_VersionMismatch value)? versionMismatch,
    TResult Function(BridgeJoinOutcome_Full value)? full,
    TResult Function(BridgeJoinOutcome_Unsafe value)? unsafe,
    TResult Function(BridgeJoinOutcome_Failed value)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeJoinOutcome_Joined() when joined != null:
        return joined(_that);
      case BridgeJoinOutcome_BadInvite() when badInvite != null:
        return badInvite(_that);
      case BridgeJoinOutcome_PasswordNeeded() when passwordNeeded != null:
        return passwordNeeded(_that);
      case BridgeJoinOutcome_Unreachable() when unreachable != null:
        return unreachable(_that);
      case BridgeJoinOutcome_VersionMismatch() when versionMismatch != null:
        return versionMismatch(_that);
      case BridgeJoinOutcome_Full() when full != null:
        return full(_that);
      case BridgeJoinOutcome_Unsafe() when unsafe != null:
        return unsafe(_that);
      case BridgeJoinOutcome_Failed() when failed != null:
        return failed(_that);
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
    required TResult Function(BridgeJoinOutcome_Joined value) joined,
    required TResult Function(BridgeJoinOutcome_BadInvite value) badInvite,
    required TResult Function(BridgeJoinOutcome_PasswordNeeded value)
        passwordNeeded,
    required TResult Function(BridgeJoinOutcome_Unreachable value) unreachable,
    required TResult Function(BridgeJoinOutcome_VersionMismatch value)
        versionMismatch,
    required TResult Function(BridgeJoinOutcome_Full value) full,
    required TResult Function(BridgeJoinOutcome_Unsafe value) unsafe,
    required TResult Function(BridgeJoinOutcome_Failed value) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeJoinOutcome_Joined():
        return joined(_that);
      case BridgeJoinOutcome_BadInvite():
        return badInvite(_that);
      case BridgeJoinOutcome_PasswordNeeded():
        return passwordNeeded(_that);
      case BridgeJoinOutcome_Unreachable():
        return unreachable(_that);
      case BridgeJoinOutcome_VersionMismatch():
        return versionMismatch(_that);
      case BridgeJoinOutcome_Full():
        return full(_that);
      case BridgeJoinOutcome_Unsafe():
        return unsafe(_that);
      case BridgeJoinOutcome_Failed():
        return failed(_that);
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
    TResult? Function(BridgeJoinOutcome_Joined value)? joined,
    TResult? Function(BridgeJoinOutcome_BadInvite value)? badInvite,
    TResult? Function(BridgeJoinOutcome_PasswordNeeded value)? passwordNeeded,
    TResult? Function(BridgeJoinOutcome_Unreachable value)? unreachable,
    TResult? Function(BridgeJoinOutcome_VersionMismatch value)? versionMismatch,
    TResult? Function(BridgeJoinOutcome_Full value)? full,
    TResult? Function(BridgeJoinOutcome_Unsafe value)? unsafe,
    TResult? Function(BridgeJoinOutcome_Failed value)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeJoinOutcome_Joined() when joined != null:
        return joined(_that);
      case BridgeJoinOutcome_BadInvite() when badInvite != null:
        return badInvite(_that);
      case BridgeJoinOutcome_PasswordNeeded() when passwordNeeded != null:
        return passwordNeeded(_that);
      case BridgeJoinOutcome_Unreachable() when unreachable != null:
        return unreachable(_that);
      case BridgeJoinOutcome_VersionMismatch() when versionMismatch != null:
        return versionMismatch(_that);
      case BridgeJoinOutcome_Full() when full != null:
        return full(_that);
      case BridgeJoinOutcome_Unsafe() when unsafe != null:
        return unsafe(_that);
      case BridgeJoinOutcome_Failed() when failed != null:
        return failed(_that);
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
    TResult Function(ProjectReference project)? joined,
    TResult Function()? badInvite,
    TResult Function()? passwordNeeded,
    TResult Function()? unreachable,
    TResult Function(String host)? versionMismatch,
    TResult Function()? full,
    TResult Function()? unsafe,
    TResult Function()? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeJoinOutcome_Joined() when joined != null:
        return joined(_that.project);
      case BridgeJoinOutcome_BadInvite() when badInvite != null:
        return badInvite();
      case BridgeJoinOutcome_PasswordNeeded() when passwordNeeded != null:
        return passwordNeeded();
      case BridgeJoinOutcome_Unreachable() when unreachable != null:
        return unreachable();
      case BridgeJoinOutcome_VersionMismatch() when versionMismatch != null:
        return versionMismatch(_that.host);
      case BridgeJoinOutcome_Full() when full != null:
        return full();
      case BridgeJoinOutcome_Unsafe() when unsafe != null:
        return unsafe();
      case BridgeJoinOutcome_Failed() when failed != null:
        return failed();
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
    required TResult Function(ProjectReference project) joined,
    required TResult Function() badInvite,
    required TResult Function() passwordNeeded,
    required TResult Function() unreachable,
    required TResult Function(String host) versionMismatch,
    required TResult Function() full,
    required TResult Function() unsafe,
    required TResult Function() failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeJoinOutcome_Joined():
        return joined(_that.project);
      case BridgeJoinOutcome_BadInvite():
        return badInvite();
      case BridgeJoinOutcome_PasswordNeeded():
        return passwordNeeded();
      case BridgeJoinOutcome_Unreachable():
        return unreachable();
      case BridgeJoinOutcome_VersionMismatch():
        return versionMismatch(_that.host);
      case BridgeJoinOutcome_Full():
        return full();
      case BridgeJoinOutcome_Unsafe():
        return unsafe();
      case BridgeJoinOutcome_Failed():
        return failed();
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
    TResult? Function(ProjectReference project)? joined,
    TResult? Function()? badInvite,
    TResult? Function()? passwordNeeded,
    TResult? Function()? unreachable,
    TResult? Function(String host)? versionMismatch,
    TResult? Function()? full,
    TResult? Function()? unsafe,
    TResult? Function()? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeJoinOutcome_Joined() when joined != null:
        return joined(_that.project);
      case BridgeJoinOutcome_BadInvite() when badInvite != null:
        return badInvite();
      case BridgeJoinOutcome_PasswordNeeded() when passwordNeeded != null:
        return passwordNeeded();
      case BridgeJoinOutcome_Unreachable() when unreachable != null:
        return unreachable();
      case BridgeJoinOutcome_VersionMismatch() when versionMismatch != null:
        return versionMismatch(_that.host);
      case BridgeJoinOutcome_Full() when full != null:
        return full();
      case BridgeJoinOutcome_Unsafe() when unsafe != null:
        return unsafe();
      case BridgeJoinOutcome_Failed() when failed != null:
        return failed();
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeJoinOutcome_Joined extends BridgeJoinOutcome {
  const BridgeJoinOutcome_Joined({required this.project}) : super._();

  final ProjectReference project;

  /// Create a copy of BridgeJoinOutcome
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeJoinOutcome_JoinedCopyWith<BridgeJoinOutcome_Joined> get copyWith =>
      _$BridgeJoinOutcome_JoinedCopyWithImpl<BridgeJoinOutcome_Joined>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeJoinOutcome_Joined &&
            (identical(other.project, project) || other.project == project));
  }

  @override
  int get hashCode => Object.hash(runtimeType, project);

  @override
  String toString() {
    return 'BridgeJoinOutcome.joined(project: $project)';
  }
}

/// @nodoc
abstract mixin class $BridgeJoinOutcome_JoinedCopyWith<$Res>
    implements $BridgeJoinOutcomeCopyWith<$Res> {
  factory $BridgeJoinOutcome_JoinedCopyWith(BridgeJoinOutcome_Joined value,
          $Res Function(BridgeJoinOutcome_Joined) _then) =
      _$BridgeJoinOutcome_JoinedCopyWithImpl;
  @useResult
  $Res call({ProjectReference project});
}

/// @nodoc
class _$BridgeJoinOutcome_JoinedCopyWithImpl<$Res>
    implements $BridgeJoinOutcome_JoinedCopyWith<$Res> {
  _$BridgeJoinOutcome_JoinedCopyWithImpl(this._self, this._then);

  final BridgeJoinOutcome_Joined _self;
  final $Res Function(BridgeJoinOutcome_Joined) _then;

  /// Create a copy of BridgeJoinOutcome
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? project = null,
  }) {
    return _then(BridgeJoinOutcome_Joined(
      project: null == project
          ? _self.project
          : project // ignore: cast_nullable_to_non_nullable
              as ProjectReference,
    ));
  }
}

/// @nodoc

class BridgeJoinOutcome_BadInvite extends BridgeJoinOutcome {
  const BridgeJoinOutcome_BadInvite() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeJoinOutcome_BadInvite);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeJoinOutcome.badInvite()';
  }
}

/// @nodoc

class BridgeJoinOutcome_PasswordNeeded extends BridgeJoinOutcome {
  const BridgeJoinOutcome_PasswordNeeded() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeJoinOutcome_PasswordNeeded);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeJoinOutcome.passwordNeeded()';
  }
}

/// @nodoc

class BridgeJoinOutcome_Unreachable extends BridgeJoinOutcome {
  const BridgeJoinOutcome_Unreachable() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeJoinOutcome_Unreachable);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeJoinOutcome.unreachable()';
  }
}

/// @nodoc

class BridgeJoinOutcome_VersionMismatch extends BridgeJoinOutcome {
  const BridgeJoinOutcome_VersionMismatch({required this.host}) : super._();

  final String host;

  /// Create a copy of BridgeJoinOutcome
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeJoinOutcome_VersionMismatchCopyWith<BridgeJoinOutcome_VersionMismatch>
      get copyWith => _$BridgeJoinOutcome_VersionMismatchCopyWithImpl<
          BridgeJoinOutcome_VersionMismatch>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeJoinOutcome_VersionMismatch &&
            (identical(other.host, host) || other.host == host));
  }

  @override
  int get hashCode => Object.hash(runtimeType, host);

  @override
  String toString() {
    return 'BridgeJoinOutcome.versionMismatch(host: $host)';
  }
}

/// @nodoc
abstract mixin class $BridgeJoinOutcome_VersionMismatchCopyWith<$Res>
    implements $BridgeJoinOutcomeCopyWith<$Res> {
  factory $BridgeJoinOutcome_VersionMismatchCopyWith(
          BridgeJoinOutcome_VersionMismatch value,
          $Res Function(BridgeJoinOutcome_VersionMismatch) _then) =
      _$BridgeJoinOutcome_VersionMismatchCopyWithImpl;
  @useResult
  $Res call({String host});
}

/// @nodoc
class _$BridgeJoinOutcome_VersionMismatchCopyWithImpl<$Res>
    implements $BridgeJoinOutcome_VersionMismatchCopyWith<$Res> {
  _$BridgeJoinOutcome_VersionMismatchCopyWithImpl(this._self, this._then);

  final BridgeJoinOutcome_VersionMismatch _self;
  final $Res Function(BridgeJoinOutcome_VersionMismatch) _then;

  /// Create a copy of BridgeJoinOutcome
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? host = null,
  }) {
    return _then(BridgeJoinOutcome_VersionMismatch(
      host: null == host
          ? _self.host
          : host // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeJoinOutcome_Full extends BridgeJoinOutcome {
  const BridgeJoinOutcome_Full() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeJoinOutcome_Full);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeJoinOutcome.full()';
  }
}

/// @nodoc

class BridgeJoinOutcome_Unsafe extends BridgeJoinOutcome {
  const BridgeJoinOutcome_Unsafe() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeJoinOutcome_Unsafe);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeJoinOutcome.unsafe()';
  }
}

/// @nodoc

class BridgeJoinOutcome_Failed extends BridgeJoinOutcome {
  const BridgeJoinOutcome_Failed() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeJoinOutcome_Failed);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeJoinOutcome.failed()';
  }
}

/// @nodoc
mixin _$BridgeShareAsking {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeShareAsking);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareAsking()';
  }
}

/// @nodoc
class $BridgeShareAskingCopyWith<$Res> {
  $BridgeShareAskingCopyWith(
      BridgeShareAsking _, $Res Function(BridgeShareAsking) __);
}

/// Adds pattern-matching-related methods to [BridgeShareAsking].
extension BridgeShareAskingPatterns on BridgeShareAsking {
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
    TResult Function(BridgeShareAsking_Idle value)? idle,
    TResult Function(BridgeShareAsking_Waiting value)? waiting,
    TResult Function(BridgeShareAsking_Running value)? running,
    TResult Function(BridgeShareAsking_Fetching value)? fetching,
    TResult Function(BridgeShareAsking_Done value)? done,
    TResult Function(BridgeShareAsking_Failed value)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareAsking_Idle() when idle != null:
        return idle(_that);
      case BridgeShareAsking_Waiting() when waiting != null:
        return waiting(_that);
      case BridgeShareAsking_Running() when running != null:
        return running(_that);
      case BridgeShareAsking_Fetching() when fetching != null:
        return fetching(_that);
      case BridgeShareAsking_Done() when done != null:
        return done(_that);
      case BridgeShareAsking_Failed() when failed != null:
        return failed(_that);
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
    required TResult Function(BridgeShareAsking_Idle value) idle,
    required TResult Function(BridgeShareAsking_Waiting value) waiting,
    required TResult Function(BridgeShareAsking_Running value) running,
    required TResult Function(BridgeShareAsking_Fetching value) fetching,
    required TResult Function(BridgeShareAsking_Done value) done,
    required TResult Function(BridgeShareAsking_Failed value) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareAsking_Idle():
        return idle(_that);
      case BridgeShareAsking_Waiting():
        return waiting(_that);
      case BridgeShareAsking_Running():
        return running(_that);
      case BridgeShareAsking_Fetching():
        return fetching(_that);
      case BridgeShareAsking_Done():
        return done(_that);
      case BridgeShareAsking_Failed():
        return failed(_that);
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
    TResult? Function(BridgeShareAsking_Idle value)? idle,
    TResult? Function(BridgeShareAsking_Waiting value)? waiting,
    TResult? Function(BridgeShareAsking_Running value)? running,
    TResult? Function(BridgeShareAsking_Fetching value)? fetching,
    TResult? Function(BridgeShareAsking_Done value)? done,
    TResult? Function(BridgeShareAsking_Failed value)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareAsking_Idle() when idle != null:
        return idle(_that);
      case BridgeShareAsking_Waiting() when waiting != null:
        return waiting(_that);
      case BridgeShareAsking_Running() when running != null:
        return running(_that);
      case BridgeShareAsking_Fetching() when fetching != null:
        return fetching(_that);
      case BridgeShareAsking_Done() when done != null:
        return done(_that);
      case BridgeShareAsking_Failed() when failed != null:
        return failed(_that);
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
    TResult Function()? idle,
    TResult Function()? waiting,
    TResult Function(BigInt frame, BigInt total)? running,
    TResult Function(BigInt done, BigInt total)? fetching,
    TResult Function(String path)? done,
    TResult Function(String why)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareAsking_Idle() when idle != null:
        return idle();
      case BridgeShareAsking_Waiting() when waiting != null:
        return waiting();
      case BridgeShareAsking_Running() when running != null:
        return running(_that.frame, _that.total);
      case BridgeShareAsking_Fetching() when fetching != null:
        return fetching(_that.done, _that.total);
      case BridgeShareAsking_Done() when done != null:
        return done(_that.path);
      case BridgeShareAsking_Failed() when failed != null:
        return failed(_that.why);
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
    required TResult Function() idle,
    required TResult Function() waiting,
    required TResult Function(BigInt frame, BigInt total) running,
    required TResult Function(BigInt done, BigInt total) fetching,
    required TResult Function(String path) done,
    required TResult Function(String why) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareAsking_Idle():
        return idle();
      case BridgeShareAsking_Waiting():
        return waiting();
      case BridgeShareAsking_Running():
        return running(_that.frame, _that.total);
      case BridgeShareAsking_Fetching():
        return fetching(_that.done, _that.total);
      case BridgeShareAsking_Done():
        return done(_that.path);
      case BridgeShareAsking_Failed():
        return failed(_that.why);
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
    TResult? Function()? idle,
    TResult? Function()? waiting,
    TResult? Function(BigInt frame, BigInt total)? running,
    TResult? Function(BigInt done, BigInt total)? fetching,
    TResult? Function(String path)? done,
    TResult? Function(String why)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareAsking_Idle() when idle != null:
        return idle();
      case BridgeShareAsking_Waiting() when waiting != null:
        return waiting();
      case BridgeShareAsking_Running() when running != null:
        return running(_that.frame, _that.total);
      case BridgeShareAsking_Fetching() when fetching != null:
        return fetching(_that.done, _that.total);
      case BridgeShareAsking_Done() when done != null:
        return done(_that.path);
      case BridgeShareAsking_Failed() when failed != null:
        return failed(_that.why);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeShareAsking_Idle extends BridgeShareAsking {
  const BridgeShareAsking_Idle() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeShareAsking_Idle);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareAsking.idle()';
  }
}

/// @nodoc

class BridgeShareAsking_Waiting extends BridgeShareAsking {
  const BridgeShareAsking_Waiting() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareAsking_Waiting);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareAsking.waiting()';
  }
}

/// @nodoc

class BridgeShareAsking_Running extends BridgeShareAsking {
  const BridgeShareAsking_Running({required this.frame, required this.total})
      : super._();

  final BigInt frame;
  final BigInt total;

  /// Create a copy of BridgeShareAsking
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareAsking_RunningCopyWith<BridgeShareAsking_Running> get copyWith =>
      _$BridgeShareAsking_RunningCopyWithImpl<BridgeShareAsking_Running>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareAsking_Running &&
            (identical(other.frame, frame) || other.frame == frame) &&
            (identical(other.total, total) || other.total == total));
  }

  @override
  int get hashCode => Object.hash(runtimeType, frame, total);

  @override
  String toString() {
    return 'BridgeShareAsking.running(frame: $frame, total: $total)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareAsking_RunningCopyWith<$Res>
    implements $BridgeShareAskingCopyWith<$Res> {
  factory $BridgeShareAsking_RunningCopyWith(BridgeShareAsking_Running value,
          $Res Function(BridgeShareAsking_Running) _then) =
      _$BridgeShareAsking_RunningCopyWithImpl;
  @useResult
  $Res call({BigInt frame, BigInt total});
}

/// @nodoc
class _$BridgeShareAsking_RunningCopyWithImpl<$Res>
    implements $BridgeShareAsking_RunningCopyWith<$Res> {
  _$BridgeShareAsking_RunningCopyWithImpl(this._self, this._then);

  final BridgeShareAsking_Running _self;
  final $Res Function(BridgeShareAsking_Running) _then;

  /// Create a copy of BridgeShareAsking
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? frame = null,
    Object? total = null,
  }) {
    return _then(BridgeShareAsking_Running(
      frame: null == frame
          ? _self.frame
          : frame // ignore: cast_nullable_to_non_nullable
              as BigInt,
      total: null == total
          ? _self.total
          : total // ignore: cast_nullable_to_non_nullable
              as BigInt,
    ));
  }
}

/// @nodoc

class BridgeShareAsking_Fetching extends BridgeShareAsking {
  const BridgeShareAsking_Fetching({required this.done, required this.total})
      : super._();

  final BigInt done;
  final BigInt total;

  /// Create a copy of BridgeShareAsking
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareAsking_FetchingCopyWith<BridgeShareAsking_Fetching>
      get copyWith =>
          _$BridgeShareAsking_FetchingCopyWithImpl<BridgeShareAsking_Fetching>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareAsking_Fetching &&
            (identical(other.done, done) || other.done == done) &&
            (identical(other.total, total) || other.total == total));
  }

  @override
  int get hashCode => Object.hash(runtimeType, done, total);

  @override
  String toString() {
    return 'BridgeShareAsking.fetching(done: $done, total: $total)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareAsking_FetchingCopyWith<$Res>
    implements $BridgeShareAskingCopyWith<$Res> {
  factory $BridgeShareAsking_FetchingCopyWith(BridgeShareAsking_Fetching value,
          $Res Function(BridgeShareAsking_Fetching) _then) =
      _$BridgeShareAsking_FetchingCopyWithImpl;
  @useResult
  $Res call({BigInt done, BigInt total});
}

/// @nodoc
class _$BridgeShareAsking_FetchingCopyWithImpl<$Res>
    implements $BridgeShareAsking_FetchingCopyWith<$Res> {
  _$BridgeShareAsking_FetchingCopyWithImpl(this._self, this._then);

  final BridgeShareAsking_Fetching _self;
  final $Res Function(BridgeShareAsking_Fetching) _then;

  /// Create a copy of BridgeShareAsking
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? done = null,
    Object? total = null,
  }) {
    return _then(BridgeShareAsking_Fetching(
      done: null == done
          ? _self.done
          : done // ignore: cast_nullable_to_non_nullable
              as BigInt,
      total: null == total
          ? _self.total
          : total // ignore: cast_nullable_to_non_nullable
              as BigInt,
    ));
  }
}

/// @nodoc

class BridgeShareAsking_Done extends BridgeShareAsking {
  const BridgeShareAsking_Done({required this.path}) : super._();

  final String path;

  /// Create a copy of BridgeShareAsking
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareAsking_DoneCopyWith<BridgeShareAsking_Done> get copyWith =>
      _$BridgeShareAsking_DoneCopyWithImpl<BridgeShareAsking_Done>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareAsking_Done &&
            (identical(other.path, path) || other.path == path));
  }

  @override
  int get hashCode => Object.hash(runtimeType, path);

  @override
  String toString() {
    return 'BridgeShareAsking.done(path: $path)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareAsking_DoneCopyWith<$Res>
    implements $BridgeShareAskingCopyWith<$Res> {
  factory $BridgeShareAsking_DoneCopyWith(BridgeShareAsking_Done value,
          $Res Function(BridgeShareAsking_Done) _then) =
      _$BridgeShareAsking_DoneCopyWithImpl;
  @useResult
  $Res call({String path});
}

/// @nodoc
class _$BridgeShareAsking_DoneCopyWithImpl<$Res>
    implements $BridgeShareAsking_DoneCopyWith<$Res> {
  _$BridgeShareAsking_DoneCopyWithImpl(this._self, this._then);

  final BridgeShareAsking_Done _self;
  final $Res Function(BridgeShareAsking_Done) _then;

  /// Create a copy of BridgeShareAsking
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? path = null,
  }) {
    return _then(BridgeShareAsking_Done(
      path: null == path
          ? _self.path
          : path // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeShareAsking_Failed extends BridgeShareAsking {
  const BridgeShareAsking_Failed({required this.why}) : super._();

  final String why;

  /// Create a copy of BridgeShareAsking
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareAsking_FailedCopyWith<BridgeShareAsking_Failed> get copyWith =>
      _$BridgeShareAsking_FailedCopyWithImpl<BridgeShareAsking_Failed>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareAsking_Failed &&
            (identical(other.why, why) || other.why == why));
  }

  @override
  int get hashCode => Object.hash(runtimeType, why);

  @override
  String toString() {
    return 'BridgeShareAsking.failed(why: $why)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareAsking_FailedCopyWith<$Res>
    implements $BridgeShareAskingCopyWith<$Res> {
  factory $BridgeShareAsking_FailedCopyWith(BridgeShareAsking_Failed value,
          $Res Function(BridgeShareAsking_Failed) _then) =
      _$BridgeShareAsking_FailedCopyWithImpl;
  @useResult
  $Res call({String why});
}

/// @nodoc
class _$BridgeShareAsking_FailedCopyWithImpl<$Res>
    implements $BridgeShareAsking_FailedCopyWith<$Res> {
  _$BridgeShareAsking_FailedCopyWithImpl(this._self, this._then);

  final BridgeShareAsking_Failed _self;
  final $Res Function(BridgeShareAsking_Failed) _then;

  /// Create a copy of BridgeShareAsking
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? why = null,
  }) {
    return _then(BridgeShareAsking_Failed(
      why: null == why
          ? _self.why
          : why // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc
mixin _$BridgeShareEnding {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeShareEnding);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareEnding()';
  }
}

/// @nodoc
class $BridgeShareEndingCopyWith<$Res> {
  $BridgeShareEndingCopyWith(
      BridgeShareEnding _, $Res Function(BridgeShareEnding) __);
}

/// Adds pattern-matching-related methods to [BridgeShareEnding].
extension BridgeShareEndingPatterns on BridgeShareEnding {
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
    TResult Function(BridgeShareEnding_Closed value)? closed,
    TResult Function(BridgeShareEnding_VersionMismatch value)? versionMismatch,
    TResult Function(BridgeShareEnding_Full value)? full,
    TResult Function(BridgeShareEnding_Unsafe value)? unsafe,
    TResult Function(BridgeShareEnding_Removed value)? removed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareEnding_Closed() when closed != null:
        return closed(_that);
      case BridgeShareEnding_VersionMismatch() when versionMismatch != null:
        return versionMismatch(_that);
      case BridgeShareEnding_Full() when full != null:
        return full(_that);
      case BridgeShareEnding_Unsafe() when unsafe != null:
        return unsafe(_that);
      case BridgeShareEnding_Removed() when removed != null:
        return removed(_that);
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
    required TResult Function(BridgeShareEnding_Closed value) closed,
    required TResult Function(BridgeShareEnding_VersionMismatch value)
        versionMismatch,
    required TResult Function(BridgeShareEnding_Full value) full,
    required TResult Function(BridgeShareEnding_Unsafe value) unsafe,
    required TResult Function(BridgeShareEnding_Removed value) removed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareEnding_Closed():
        return closed(_that);
      case BridgeShareEnding_VersionMismatch():
        return versionMismatch(_that);
      case BridgeShareEnding_Full():
        return full(_that);
      case BridgeShareEnding_Unsafe():
        return unsafe(_that);
      case BridgeShareEnding_Removed():
        return removed(_that);
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
    TResult? Function(BridgeShareEnding_Closed value)? closed,
    TResult? Function(BridgeShareEnding_VersionMismatch value)? versionMismatch,
    TResult? Function(BridgeShareEnding_Full value)? full,
    TResult? Function(BridgeShareEnding_Unsafe value)? unsafe,
    TResult? Function(BridgeShareEnding_Removed value)? removed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareEnding_Closed() when closed != null:
        return closed(_that);
      case BridgeShareEnding_VersionMismatch() when versionMismatch != null:
        return versionMismatch(_that);
      case BridgeShareEnding_Full() when full != null:
        return full(_that);
      case BridgeShareEnding_Unsafe() when unsafe != null:
        return unsafe(_that);
      case BridgeShareEnding_Removed() when removed != null:
        return removed(_that);
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
    TResult Function()? closed,
    TResult Function(String host)? versionMismatch,
    TResult Function()? full,
    TResult Function()? unsafe,
    TResult Function()? removed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareEnding_Closed() when closed != null:
        return closed();
      case BridgeShareEnding_VersionMismatch() when versionMismatch != null:
        return versionMismatch(_that.host);
      case BridgeShareEnding_Full() when full != null:
        return full();
      case BridgeShareEnding_Unsafe() when unsafe != null:
        return unsafe();
      case BridgeShareEnding_Removed() when removed != null:
        return removed();
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
    required TResult Function() closed,
    required TResult Function(String host) versionMismatch,
    required TResult Function() full,
    required TResult Function() unsafe,
    required TResult Function() removed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareEnding_Closed():
        return closed();
      case BridgeShareEnding_VersionMismatch():
        return versionMismatch(_that.host);
      case BridgeShareEnding_Full():
        return full();
      case BridgeShareEnding_Unsafe():
        return unsafe();
      case BridgeShareEnding_Removed():
        return removed();
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
    TResult? Function()? closed,
    TResult? Function(String host)? versionMismatch,
    TResult? Function()? full,
    TResult? Function()? unsafe,
    TResult? Function()? removed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareEnding_Closed() when closed != null:
        return closed();
      case BridgeShareEnding_VersionMismatch() when versionMismatch != null:
        return versionMismatch(_that.host);
      case BridgeShareEnding_Full() when full != null:
        return full();
      case BridgeShareEnding_Unsafe() when unsafe != null:
        return unsafe();
      case BridgeShareEnding_Removed() when removed != null:
        return removed();
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeShareEnding_Closed extends BridgeShareEnding {
  const BridgeShareEnding_Closed() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeShareEnding_Closed);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareEnding.closed()';
  }
}

/// @nodoc

class BridgeShareEnding_VersionMismatch extends BridgeShareEnding {
  const BridgeShareEnding_VersionMismatch({required this.host}) : super._();

  final String host;

  /// Create a copy of BridgeShareEnding
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareEnding_VersionMismatchCopyWith<BridgeShareEnding_VersionMismatch>
      get copyWith => _$BridgeShareEnding_VersionMismatchCopyWithImpl<
          BridgeShareEnding_VersionMismatch>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEnding_VersionMismatch &&
            (identical(other.host, host) || other.host == host));
  }

  @override
  int get hashCode => Object.hash(runtimeType, host);

  @override
  String toString() {
    return 'BridgeShareEnding.versionMismatch(host: $host)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareEnding_VersionMismatchCopyWith<$Res>
    implements $BridgeShareEndingCopyWith<$Res> {
  factory $BridgeShareEnding_VersionMismatchCopyWith(
          BridgeShareEnding_VersionMismatch value,
          $Res Function(BridgeShareEnding_VersionMismatch) _then) =
      _$BridgeShareEnding_VersionMismatchCopyWithImpl;
  @useResult
  $Res call({String host});
}

/// @nodoc
class _$BridgeShareEnding_VersionMismatchCopyWithImpl<$Res>
    implements $BridgeShareEnding_VersionMismatchCopyWith<$Res> {
  _$BridgeShareEnding_VersionMismatchCopyWithImpl(this._self, this._then);

  final BridgeShareEnding_VersionMismatch _self;
  final $Res Function(BridgeShareEnding_VersionMismatch) _then;

  /// Create a copy of BridgeShareEnding
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? host = null,
  }) {
    return _then(BridgeShareEnding_VersionMismatch(
      host: null == host
          ? _self.host
          : host // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeShareEnding_Full extends BridgeShareEnding {
  const BridgeShareEnding_Full() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeShareEnding_Full);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareEnding.full()';
  }
}

/// @nodoc

class BridgeShareEnding_Unsafe extends BridgeShareEnding {
  const BridgeShareEnding_Unsafe() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeShareEnding_Unsafe);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareEnding.unsafe()';
  }
}

/// @nodoc

class BridgeShareEnding_Removed extends BridgeShareEnding {
  const BridgeShareEnding_Removed() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEnding_Removed);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareEnding.removed()';
  }
}

/// @nodoc
mixin _$BridgeShareEvent {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeShareEvent);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareEvent()';
  }
}

/// @nodoc
class $BridgeShareEventCopyWith<$Res> {
  $BridgeShareEventCopyWith(
      BridgeShareEvent _, $Res Function(BridgeShareEvent) __);
}

/// Adds pattern-matching-related methods to [BridgeShareEvent].
extension BridgeShareEventPatterns on BridgeShareEvent {
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
    TResult Function(BridgeShareEvent_People value)? people,
    TResult Function(BridgeShareEvent_Away value)? away,
    TResult Function(BridgeShareEvent_Back value)? back,
    TResult Function(BridgeShareEvent_Elsewhere value)? elsewhere,
    TResult Function(BridgeShareEvent_Ended value)? ended,
    TResult Function(BridgeShareEvent_Reach value)? reach,
    TResult Function(BridgeShareEvent_Relayed value)? relayed,
    TResult Function(BridgeShareEvent_Footage value)? footage,
    TResult Function(BridgeShareEvent_ExportAsked value)? exportAsked,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareEvent_People() when people != null:
        return people(_that);
      case BridgeShareEvent_Away() when away != null:
        return away(_that);
      case BridgeShareEvent_Back() when back != null:
        return back(_that);
      case BridgeShareEvent_Elsewhere() when elsewhere != null:
        return elsewhere(_that);
      case BridgeShareEvent_Ended() when ended != null:
        return ended(_that);
      case BridgeShareEvent_Reach() when reach != null:
        return reach(_that);
      case BridgeShareEvent_Relayed() when relayed != null:
        return relayed(_that);
      case BridgeShareEvent_Footage() when footage != null:
        return footage(_that);
      case BridgeShareEvent_ExportAsked() when exportAsked != null:
        return exportAsked(_that);
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
    required TResult Function(BridgeShareEvent_People value) people,
    required TResult Function(BridgeShareEvent_Away value) away,
    required TResult Function(BridgeShareEvent_Back value) back,
    required TResult Function(BridgeShareEvent_Elsewhere value) elsewhere,
    required TResult Function(BridgeShareEvent_Ended value) ended,
    required TResult Function(BridgeShareEvent_Reach value) reach,
    required TResult Function(BridgeShareEvent_Relayed value) relayed,
    required TResult Function(BridgeShareEvent_Footage value) footage,
    required TResult Function(BridgeShareEvent_ExportAsked value) exportAsked,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareEvent_People():
        return people(_that);
      case BridgeShareEvent_Away():
        return away(_that);
      case BridgeShareEvent_Back():
        return back(_that);
      case BridgeShareEvent_Elsewhere():
        return elsewhere(_that);
      case BridgeShareEvent_Ended():
        return ended(_that);
      case BridgeShareEvent_Reach():
        return reach(_that);
      case BridgeShareEvent_Relayed():
        return relayed(_that);
      case BridgeShareEvent_Footage():
        return footage(_that);
      case BridgeShareEvent_ExportAsked():
        return exportAsked(_that);
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
    TResult? Function(BridgeShareEvent_People value)? people,
    TResult? Function(BridgeShareEvent_Away value)? away,
    TResult? Function(BridgeShareEvent_Back value)? back,
    TResult? Function(BridgeShareEvent_Elsewhere value)? elsewhere,
    TResult? Function(BridgeShareEvent_Ended value)? ended,
    TResult? Function(BridgeShareEvent_Reach value)? reach,
    TResult? Function(BridgeShareEvent_Relayed value)? relayed,
    TResult? Function(BridgeShareEvent_Footage value)? footage,
    TResult? Function(BridgeShareEvent_ExportAsked value)? exportAsked,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareEvent_People() when people != null:
        return people(_that);
      case BridgeShareEvent_Away() when away != null:
        return away(_that);
      case BridgeShareEvent_Back() when back != null:
        return back(_that);
      case BridgeShareEvent_Elsewhere() when elsewhere != null:
        return elsewhere(_that);
      case BridgeShareEvent_Ended() when ended != null:
        return ended(_that);
      case BridgeShareEvent_Reach() when reach != null:
        return reach(_that);
      case BridgeShareEvent_Relayed() when relayed != null:
        return relayed(_that);
      case BridgeShareEvent_Footage() when footage != null:
        return footage(_that);
      case BridgeShareEvent_ExportAsked() when exportAsked != null:
        return exportAsked(_that);
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
    TResult Function(List<BridgeSharePerson> people)? people,
    TResult Function()? away,
    TResult Function(int held, int refused)? back,
    TResult Function()? elsewhere,
    TResult Function(BridgeShareEnding reason)? ended,
    TResult Function(BridgeShareReach reach)? reach,
    TResult Function(BridgeShareRelayed relayed)? relayed,
    TResult Function(bool placed)? footage,
    TResult Function(String job, int from, String comp)? exportAsked,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareEvent_People() when people != null:
        return people(_that.people);
      case BridgeShareEvent_Away() when away != null:
        return away();
      case BridgeShareEvent_Back() when back != null:
        return back(_that.held, _that.refused);
      case BridgeShareEvent_Elsewhere() when elsewhere != null:
        return elsewhere();
      case BridgeShareEvent_Ended() when ended != null:
        return ended(_that.reason);
      case BridgeShareEvent_Reach() when reach != null:
        return reach(_that.reach);
      case BridgeShareEvent_Relayed() when relayed != null:
        return relayed(_that.relayed);
      case BridgeShareEvent_Footage() when footage != null:
        return footage(_that.placed);
      case BridgeShareEvent_ExportAsked() when exportAsked != null:
        return exportAsked(_that.job, _that.from, _that.comp);
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
    required TResult Function(List<BridgeSharePerson> people) people,
    required TResult Function() away,
    required TResult Function(int held, int refused) back,
    required TResult Function() elsewhere,
    required TResult Function(BridgeShareEnding reason) ended,
    required TResult Function(BridgeShareReach reach) reach,
    required TResult Function(BridgeShareRelayed relayed) relayed,
    required TResult Function(bool placed) footage,
    required TResult Function(String job, int from, String comp) exportAsked,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareEvent_People():
        return people(_that.people);
      case BridgeShareEvent_Away():
        return away();
      case BridgeShareEvent_Back():
        return back(_that.held, _that.refused);
      case BridgeShareEvent_Elsewhere():
        return elsewhere();
      case BridgeShareEvent_Ended():
        return ended(_that.reason);
      case BridgeShareEvent_Reach():
        return reach(_that.reach);
      case BridgeShareEvent_Relayed():
        return relayed(_that.relayed);
      case BridgeShareEvent_Footage():
        return footage(_that.placed);
      case BridgeShareEvent_ExportAsked():
        return exportAsked(_that.job, _that.from, _that.comp);
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
    TResult? Function(List<BridgeSharePerson> people)? people,
    TResult? Function()? away,
    TResult? Function(int held, int refused)? back,
    TResult? Function()? elsewhere,
    TResult? Function(BridgeShareEnding reason)? ended,
    TResult? Function(BridgeShareReach reach)? reach,
    TResult? Function(BridgeShareRelayed relayed)? relayed,
    TResult? Function(bool placed)? footage,
    TResult? Function(String job, int from, String comp)? exportAsked,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareEvent_People() when people != null:
        return people(_that.people);
      case BridgeShareEvent_Away() when away != null:
        return away();
      case BridgeShareEvent_Back() when back != null:
        return back(_that.held, _that.refused);
      case BridgeShareEvent_Elsewhere() when elsewhere != null:
        return elsewhere();
      case BridgeShareEvent_Ended() when ended != null:
        return ended(_that.reason);
      case BridgeShareEvent_Reach() when reach != null:
        return reach(_that.reach);
      case BridgeShareEvent_Relayed() when relayed != null:
        return relayed(_that.relayed);
      case BridgeShareEvent_Footage() when footage != null:
        return footage(_that.placed);
      case BridgeShareEvent_ExportAsked() when exportAsked != null:
        return exportAsked(_that.job, _that.from, _that.comp);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeShareEvent_People extends BridgeShareEvent {
  const BridgeShareEvent_People({required final List<BridgeSharePerson> people})
      : _people = people,
        super._();

  final List<BridgeSharePerson> _people;
  List<BridgeSharePerson> get people {
    if (_people is EqualUnmodifiableListView) return _people;
    // ignore: implicit_dynamic_type
    return EqualUnmodifiableListView(_people);
  }

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareEvent_PeopleCopyWith<BridgeShareEvent_People> get copyWith =>
      _$BridgeShareEvent_PeopleCopyWithImpl<BridgeShareEvent_People>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEvent_People &&
            const DeepCollectionEquality().equals(other._people, _people));
  }

  @override
  int get hashCode =>
      Object.hash(runtimeType, const DeepCollectionEquality().hash(_people));

  @override
  String toString() {
    return 'BridgeShareEvent.people(people: $people)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareEvent_PeopleCopyWith<$Res>
    implements $BridgeShareEventCopyWith<$Res> {
  factory $BridgeShareEvent_PeopleCopyWith(BridgeShareEvent_People value,
          $Res Function(BridgeShareEvent_People) _then) =
      _$BridgeShareEvent_PeopleCopyWithImpl;
  @useResult
  $Res call({List<BridgeSharePerson> people});
}

/// @nodoc
class _$BridgeShareEvent_PeopleCopyWithImpl<$Res>
    implements $BridgeShareEvent_PeopleCopyWith<$Res> {
  _$BridgeShareEvent_PeopleCopyWithImpl(this._self, this._then);

  final BridgeShareEvent_People _self;
  final $Res Function(BridgeShareEvent_People) _then;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? people = null,
  }) {
    return _then(BridgeShareEvent_People(
      people: null == people
          ? _self._people
          : people // ignore: cast_nullable_to_non_nullable
              as List<BridgeSharePerson>,
    ));
  }
}

/// @nodoc

class BridgeShareEvent_Away extends BridgeShareEvent {
  const BridgeShareEvent_Away() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeShareEvent_Away);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareEvent.away()';
  }
}

/// @nodoc

class BridgeShareEvent_Back extends BridgeShareEvent {
  const BridgeShareEvent_Back({required this.held, required this.refused})
      : super._();

  final int held;
  final int refused;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareEvent_BackCopyWith<BridgeShareEvent_Back> get copyWith =>
      _$BridgeShareEvent_BackCopyWithImpl<BridgeShareEvent_Back>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEvent_Back &&
            (identical(other.held, held) || other.held == held) &&
            (identical(other.refused, refused) || other.refused == refused));
  }

  @override
  int get hashCode => Object.hash(runtimeType, held, refused);

  @override
  String toString() {
    return 'BridgeShareEvent.back(held: $held, refused: $refused)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareEvent_BackCopyWith<$Res>
    implements $BridgeShareEventCopyWith<$Res> {
  factory $BridgeShareEvent_BackCopyWith(BridgeShareEvent_Back value,
          $Res Function(BridgeShareEvent_Back) _then) =
      _$BridgeShareEvent_BackCopyWithImpl;
  @useResult
  $Res call({int held, int refused});
}

/// @nodoc
class _$BridgeShareEvent_BackCopyWithImpl<$Res>
    implements $BridgeShareEvent_BackCopyWith<$Res> {
  _$BridgeShareEvent_BackCopyWithImpl(this._self, this._then);

  final BridgeShareEvent_Back _self;
  final $Res Function(BridgeShareEvent_Back) _then;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? held = null,
    Object? refused = null,
  }) {
    return _then(BridgeShareEvent_Back(
      held: null == held
          ? _self.held
          : held // ignore: cast_nullable_to_non_nullable
              as int,
      refused: null == refused
          ? _self.refused
          : refused // ignore: cast_nullable_to_non_nullable
              as int,
    ));
  }
}

/// @nodoc

class BridgeShareEvent_Elsewhere extends BridgeShareEvent {
  const BridgeShareEvent_Elsewhere() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEvent_Elsewhere);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareEvent.elsewhere()';
  }
}

/// @nodoc

class BridgeShareEvent_Ended extends BridgeShareEvent {
  const BridgeShareEvent_Ended({required this.reason}) : super._();

  final BridgeShareEnding reason;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareEvent_EndedCopyWith<BridgeShareEvent_Ended> get copyWith =>
      _$BridgeShareEvent_EndedCopyWithImpl<BridgeShareEvent_Ended>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEvent_Ended &&
            (identical(other.reason, reason) || other.reason == reason));
  }

  @override
  int get hashCode => Object.hash(runtimeType, reason);

  @override
  String toString() {
    return 'BridgeShareEvent.ended(reason: $reason)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareEvent_EndedCopyWith<$Res>
    implements $BridgeShareEventCopyWith<$Res> {
  factory $BridgeShareEvent_EndedCopyWith(BridgeShareEvent_Ended value,
          $Res Function(BridgeShareEvent_Ended) _then) =
      _$BridgeShareEvent_EndedCopyWithImpl;
  @useResult
  $Res call({BridgeShareEnding reason});

  $BridgeShareEndingCopyWith<$Res> get reason;
}

/// @nodoc
class _$BridgeShareEvent_EndedCopyWithImpl<$Res>
    implements $BridgeShareEvent_EndedCopyWith<$Res> {
  _$BridgeShareEvent_EndedCopyWithImpl(this._self, this._then);

  final BridgeShareEvent_Ended _self;
  final $Res Function(BridgeShareEvent_Ended) _then;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? reason = null,
  }) {
    return _then(BridgeShareEvent_Ended(
      reason: null == reason
          ? _self.reason
          : reason // ignore: cast_nullable_to_non_nullable
              as BridgeShareEnding,
    ));
  }

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @override
  @pragma('vm:prefer-inline')
  $BridgeShareEndingCopyWith<$Res> get reason {
    return $BridgeShareEndingCopyWith<$Res>(_self.reason, (value) {
      return _then(_self.copyWith(reason: value));
    });
  }
}

/// @nodoc

class BridgeShareEvent_Reach extends BridgeShareEvent {
  const BridgeShareEvent_Reach({required this.reach}) : super._();

  final BridgeShareReach reach;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareEvent_ReachCopyWith<BridgeShareEvent_Reach> get copyWith =>
      _$BridgeShareEvent_ReachCopyWithImpl<BridgeShareEvent_Reach>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEvent_Reach &&
            (identical(other.reach, reach) || other.reach == reach));
  }

  @override
  int get hashCode => Object.hash(runtimeType, reach);

  @override
  String toString() {
    return 'BridgeShareEvent.reach(reach: $reach)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareEvent_ReachCopyWith<$Res>
    implements $BridgeShareEventCopyWith<$Res> {
  factory $BridgeShareEvent_ReachCopyWith(BridgeShareEvent_Reach value,
          $Res Function(BridgeShareEvent_Reach) _then) =
      _$BridgeShareEvent_ReachCopyWithImpl;
  @useResult
  $Res call({BridgeShareReach reach});

  $BridgeShareReachCopyWith<$Res> get reach;
}

/// @nodoc
class _$BridgeShareEvent_ReachCopyWithImpl<$Res>
    implements $BridgeShareEvent_ReachCopyWith<$Res> {
  _$BridgeShareEvent_ReachCopyWithImpl(this._self, this._then);

  final BridgeShareEvent_Reach _self;
  final $Res Function(BridgeShareEvent_Reach) _then;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? reach = null,
  }) {
    return _then(BridgeShareEvent_Reach(
      reach: null == reach
          ? _self.reach
          : reach // ignore: cast_nullable_to_non_nullable
              as BridgeShareReach,
    ));
  }

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @override
  @pragma('vm:prefer-inline')
  $BridgeShareReachCopyWith<$Res> get reach {
    return $BridgeShareReachCopyWith<$Res>(_self.reach, (value) {
      return _then(_self.copyWith(reach: value));
    });
  }
}

/// @nodoc

class BridgeShareEvent_Relayed extends BridgeShareEvent {
  const BridgeShareEvent_Relayed({required this.relayed}) : super._();

  final BridgeShareRelayed relayed;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareEvent_RelayedCopyWith<BridgeShareEvent_Relayed> get copyWith =>
      _$BridgeShareEvent_RelayedCopyWithImpl<BridgeShareEvent_Relayed>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEvent_Relayed &&
            (identical(other.relayed, relayed) || other.relayed == relayed));
  }

  @override
  int get hashCode => Object.hash(runtimeType, relayed);

  @override
  String toString() {
    return 'BridgeShareEvent.relayed(relayed: $relayed)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareEvent_RelayedCopyWith<$Res>
    implements $BridgeShareEventCopyWith<$Res> {
  factory $BridgeShareEvent_RelayedCopyWith(BridgeShareEvent_Relayed value,
          $Res Function(BridgeShareEvent_Relayed) _then) =
      _$BridgeShareEvent_RelayedCopyWithImpl;
  @useResult
  $Res call({BridgeShareRelayed relayed});
}

/// @nodoc
class _$BridgeShareEvent_RelayedCopyWithImpl<$Res>
    implements $BridgeShareEvent_RelayedCopyWith<$Res> {
  _$BridgeShareEvent_RelayedCopyWithImpl(this._self, this._then);

  final BridgeShareEvent_Relayed _self;
  final $Res Function(BridgeShareEvent_Relayed) _then;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? relayed = null,
  }) {
    return _then(BridgeShareEvent_Relayed(
      relayed: null == relayed
          ? _self.relayed
          : relayed // ignore: cast_nullable_to_non_nullable
              as BridgeShareRelayed,
    ));
  }
}

/// @nodoc

class BridgeShareEvent_Footage extends BridgeShareEvent {
  const BridgeShareEvent_Footage({required this.placed}) : super._();

  final bool placed;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareEvent_FootageCopyWith<BridgeShareEvent_Footage> get copyWith =>
      _$BridgeShareEvent_FootageCopyWithImpl<BridgeShareEvent_Footage>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEvent_Footage &&
            (identical(other.placed, placed) || other.placed == placed));
  }

  @override
  int get hashCode => Object.hash(runtimeType, placed);

  @override
  String toString() {
    return 'BridgeShareEvent.footage(placed: $placed)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareEvent_FootageCopyWith<$Res>
    implements $BridgeShareEventCopyWith<$Res> {
  factory $BridgeShareEvent_FootageCopyWith(BridgeShareEvent_Footage value,
          $Res Function(BridgeShareEvent_Footage) _then) =
      _$BridgeShareEvent_FootageCopyWithImpl;
  @useResult
  $Res call({bool placed});
}

/// @nodoc
class _$BridgeShareEvent_FootageCopyWithImpl<$Res>
    implements $BridgeShareEvent_FootageCopyWith<$Res> {
  _$BridgeShareEvent_FootageCopyWithImpl(this._self, this._then);

  final BridgeShareEvent_Footage _self;
  final $Res Function(BridgeShareEvent_Footage) _then;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? placed = null,
  }) {
    return _then(BridgeShareEvent_Footage(
      placed: null == placed
          ? _self.placed
          : placed // ignore: cast_nullable_to_non_nullable
              as bool,
    ));
  }
}

/// @nodoc

class BridgeShareEvent_ExportAsked extends BridgeShareEvent {
  const BridgeShareEvent_ExportAsked(
      {required this.job, required this.from, required this.comp})
      : super._();

  final String job;
  final int from;
  final String comp;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareEvent_ExportAskedCopyWith<BridgeShareEvent_ExportAsked>
      get copyWith => _$BridgeShareEvent_ExportAskedCopyWithImpl<
          BridgeShareEvent_ExportAsked>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEvent_ExportAsked &&
            (identical(other.job, job) || other.job == job) &&
            (identical(other.from, from) || other.from == from) &&
            (identical(other.comp, comp) || other.comp == comp));
  }

  @override
  int get hashCode => Object.hash(runtimeType, job, from, comp);

  @override
  String toString() {
    return 'BridgeShareEvent.exportAsked(job: $job, from: $from, comp: $comp)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareEvent_ExportAskedCopyWith<$Res>
    implements $BridgeShareEventCopyWith<$Res> {
  factory $BridgeShareEvent_ExportAskedCopyWith(
          BridgeShareEvent_ExportAsked value,
          $Res Function(BridgeShareEvent_ExportAsked) _then) =
      _$BridgeShareEvent_ExportAskedCopyWithImpl;
  @useResult
  $Res call({String job, int from, String comp});
}

/// @nodoc
class _$BridgeShareEvent_ExportAskedCopyWithImpl<$Res>
    implements $BridgeShareEvent_ExportAskedCopyWith<$Res> {
  _$BridgeShareEvent_ExportAskedCopyWithImpl(this._self, this._then);

  final BridgeShareEvent_ExportAsked _self;
  final $Res Function(BridgeShareEvent_ExportAsked) _then;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? job = null,
    Object? from = null,
    Object? comp = null,
  }) {
    return _then(BridgeShareEvent_ExportAsked(
      job: null == job
          ? _self.job
          : job // ignore: cast_nullable_to_non_nullable
              as String,
      from: null == from
          ? _self.from
          : from // ignore: cast_nullable_to_non_nullable
              as int,
      comp: null == comp
          ? _self.comp
          : comp // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc
mixin _$BridgeShareFetching {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeShareFetching);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareFetching()';
  }
}

/// @nodoc
class $BridgeShareFetchingCopyWith<$Res> {
  $BridgeShareFetchingCopyWith(
      BridgeShareFetching _, $Res Function(BridgeShareFetching) __);
}

/// Adds pattern-matching-related methods to [BridgeShareFetching].
extension BridgeShareFetchingPatterns on BridgeShareFetching {
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
    TResult Function(BridgeShareFetching_Idle value)? idle,
    TResult Function(BridgeShareFetching_Working value)? working,
    TResult Function(BridgeShareFetching_Queued value)? queued,
    TResult Function(BridgeShareFetching_Failed value)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareFetching_Idle() when idle != null:
        return idle(_that);
      case BridgeShareFetching_Working() when working != null:
        return working(_that);
      case BridgeShareFetching_Queued() when queued != null:
        return queued(_that);
      case BridgeShareFetching_Failed() when failed != null:
        return failed(_that);
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
    required TResult Function(BridgeShareFetching_Idle value) idle,
    required TResult Function(BridgeShareFetching_Working value) working,
    required TResult Function(BridgeShareFetching_Queued value) queued,
    required TResult Function(BridgeShareFetching_Failed value) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareFetching_Idle():
        return idle(_that);
      case BridgeShareFetching_Working():
        return working(_that);
      case BridgeShareFetching_Queued():
        return queued(_that);
      case BridgeShareFetching_Failed():
        return failed(_that);
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
    TResult? Function(BridgeShareFetching_Idle value)? idle,
    TResult? Function(BridgeShareFetching_Working value)? working,
    TResult? Function(BridgeShareFetching_Queued value)? queued,
    TResult? Function(BridgeShareFetching_Failed value)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareFetching_Idle() when idle != null:
        return idle(_that);
      case BridgeShareFetching_Working() when working != null:
        return working(_that);
      case BridgeShareFetching_Queued() when queued != null:
        return queued(_that);
      case BridgeShareFetching_Failed() when failed != null:
        return failed(_that);
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
    TResult Function()? idle,
    TResult Function(BigInt done, BigInt total)? working,
    TResult Function(int id)? queued,
    TResult Function()? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareFetching_Idle() when idle != null:
        return idle();
      case BridgeShareFetching_Working() when working != null:
        return working(_that.done, _that.total);
      case BridgeShareFetching_Queued() when queued != null:
        return queued(_that.id);
      case BridgeShareFetching_Failed() when failed != null:
        return failed();
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
    required TResult Function() idle,
    required TResult Function(BigInt done, BigInt total) working,
    required TResult Function(int id) queued,
    required TResult Function() failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareFetching_Idle():
        return idle();
      case BridgeShareFetching_Working():
        return working(_that.done, _that.total);
      case BridgeShareFetching_Queued():
        return queued(_that.id);
      case BridgeShareFetching_Failed():
        return failed();
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
    TResult? Function()? idle,
    TResult? Function(BigInt done, BigInt total)? working,
    TResult? Function(int id)? queued,
    TResult? Function()? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareFetching_Idle() when idle != null:
        return idle();
      case BridgeShareFetching_Working() when working != null:
        return working(_that.done, _that.total);
      case BridgeShareFetching_Queued() when queued != null:
        return queued(_that.id);
      case BridgeShareFetching_Failed() when failed != null:
        return failed();
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeShareFetching_Idle extends BridgeShareFetching {
  const BridgeShareFetching_Idle() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeShareFetching_Idle);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareFetching.idle()';
  }
}

/// @nodoc

class BridgeShareFetching_Working extends BridgeShareFetching {
  const BridgeShareFetching_Working({required this.done, required this.total})
      : super._();

  final BigInt done;
  final BigInt total;

  /// Create a copy of BridgeShareFetching
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareFetching_WorkingCopyWith<BridgeShareFetching_Working>
      get copyWith => _$BridgeShareFetching_WorkingCopyWithImpl<
          BridgeShareFetching_Working>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareFetching_Working &&
            (identical(other.done, done) || other.done == done) &&
            (identical(other.total, total) || other.total == total));
  }

  @override
  int get hashCode => Object.hash(runtimeType, done, total);

  @override
  String toString() {
    return 'BridgeShareFetching.working(done: $done, total: $total)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareFetching_WorkingCopyWith<$Res>
    implements $BridgeShareFetchingCopyWith<$Res> {
  factory $BridgeShareFetching_WorkingCopyWith(
          BridgeShareFetching_Working value,
          $Res Function(BridgeShareFetching_Working) _then) =
      _$BridgeShareFetching_WorkingCopyWithImpl;
  @useResult
  $Res call({BigInt done, BigInt total});
}

/// @nodoc
class _$BridgeShareFetching_WorkingCopyWithImpl<$Res>
    implements $BridgeShareFetching_WorkingCopyWith<$Res> {
  _$BridgeShareFetching_WorkingCopyWithImpl(this._self, this._then);

  final BridgeShareFetching_Working _self;
  final $Res Function(BridgeShareFetching_Working) _then;

  /// Create a copy of BridgeShareFetching
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? done = null,
    Object? total = null,
  }) {
    return _then(BridgeShareFetching_Working(
      done: null == done
          ? _self.done
          : done // ignore: cast_nullable_to_non_nullable
              as BigInt,
      total: null == total
          ? _self.total
          : total // ignore: cast_nullable_to_non_nullable
              as BigInt,
    ));
  }
}

/// @nodoc

class BridgeShareFetching_Queued extends BridgeShareFetching {
  const BridgeShareFetching_Queued({required this.id}) : super._();

  final int id;

  /// Create a copy of BridgeShareFetching
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareFetching_QueuedCopyWith<BridgeShareFetching_Queued>
      get copyWith =>
          _$BridgeShareFetching_QueuedCopyWithImpl<BridgeShareFetching_Queued>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareFetching_Queued &&
            (identical(other.id, id) || other.id == id));
  }

  @override
  int get hashCode => Object.hash(runtimeType, id);

  @override
  String toString() {
    return 'BridgeShareFetching.queued(id: $id)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareFetching_QueuedCopyWith<$Res>
    implements $BridgeShareFetchingCopyWith<$Res> {
  factory $BridgeShareFetching_QueuedCopyWith(BridgeShareFetching_Queued value,
          $Res Function(BridgeShareFetching_Queued) _then) =
      _$BridgeShareFetching_QueuedCopyWithImpl;
  @useResult
  $Res call({int id});
}

/// @nodoc
class _$BridgeShareFetching_QueuedCopyWithImpl<$Res>
    implements $BridgeShareFetching_QueuedCopyWith<$Res> {
  _$BridgeShareFetching_QueuedCopyWithImpl(this._self, this._then);

  final BridgeShareFetching_Queued _self;
  final $Res Function(BridgeShareFetching_Queued) _then;

  /// Create a copy of BridgeShareFetching
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? id = null,
  }) {
    return _then(BridgeShareFetching_Queued(
      id: null == id
          ? _self.id
          : id // ignore: cast_nullable_to_non_nullable
              as int,
    ));
  }
}

/// @nodoc

class BridgeShareFetching_Failed extends BridgeShareFetching {
  const BridgeShareFetching_Failed() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareFetching_Failed);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareFetching.failed()';
  }
}

/// @nodoc
mixin _$BridgeShareReach {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeShareReach);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareReach()';
  }
}

/// @nodoc
class $BridgeShareReachCopyWith<$Res> {
  $BridgeShareReachCopyWith(
      BridgeShareReach _, $Res Function(BridgeShareReach) __);
}

/// Adds pattern-matching-related methods to [BridgeShareReach].
extension BridgeShareReachPatterns on BridgeShareReach {
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
    TResult Function(BridgeShareReach_Off value)? off,
    TResult Function(BridgeShareReach_Asking value)? asking,
    TResult Function(BridgeShareReach_Open value)? open,
    TResult Function(BridgeShareReach_Refused value)? refused,
    TResult Function(BridgeShareReach_Behind value)? behind,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareReach_Off() when off != null:
        return off(_that);
      case BridgeShareReach_Asking() when asking != null:
        return asking(_that);
      case BridgeShareReach_Open() when open != null:
        return open(_that);
      case BridgeShareReach_Refused() when refused != null:
        return refused(_that);
      case BridgeShareReach_Behind() when behind != null:
        return behind(_that);
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
    required TResult Function(BridgeShareReach_Off value) off,
    required TResult Function(BridgeShareReach_Asking value) asking,
    required TResult Function(BridgeShareReach_Open value) open,
    required TResult Function(BridgeShareReach_Refused value) refused,
    required TResult Function(BridgeShareReach_Behind value) behind,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareReach_Off():
        return off(_that);
      case BridgeShareReach_Asking():
        return asking(_that);
      case BridgeShareReach_Open():
        return open(_that);
      case BridgeShareReach_Refused():
        return refused(_that);
      case BridgeShareReach_Behind():
        return behind(_that);
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
    TResult? Function(BridgeShareReach_Off value)? off,
    TResult? Function(BridgeShareReach_Asking value)? asking,
    TResult? Function(BridgeShareReach_Open value)? open,
    TResult? Function(BridgeShareReach_Refused value)? refused,
    TResult? Function(BridgeShareReach_Behind value)? behind,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareReach_Off() when off != null:
        return off(_that);
      case BridgeShareReach_Asking() when asking != null:
        return asking(_that);
      case BridgeShareReach_Open() when open != null:
        return open(_that);
      case BridgeShareReach_Refused() when refused != null:
        return refused(_that);
      case BridgeShareReach_Behind() when behind != null:
        return behind(_that);
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
    TResult Function()? asking,
    TResult Function(String address)? open,
    TResult Function()? refused,
    TResult Function()? behind,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareReach_Off() when off != null:
        return off();
      case BridgeShareReach_Asking() when asking != null:
        return asking();
      case BridgeShareReach_Open() when open != null:
        return open(_that.address);
      case BridgeShareReach_Refused() when refused != null:
        return refused();
      case BridgeShareReach_Behind() when behind != null:
        return behind();
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
    required TResult Function() asking,
    required TResult Function(String address) open,
    required TResult Function() refused,
    required TResult Function() behind,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareReach_Off():
        return off();
      case BridgeShareReach_Asking():
        return asking();
      case BridgeShareReach_Open():
        return open(_that.address);
      case BridgeShareReach_Refused():
        return refused();
      case BridgeShareReach_Behind():
        return behind();
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
    TResult? Function()? asking,
    TResult? Function(String address)? open,
    TResult? Function()? refused,
    TResult? Function()? behind,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareReach_Off() when off != null:
        return off();
      case BridgeShareReach_Asking() when asking != null:
        return asking();
      case BridgeShareReach_Open() when open != null:
        return open(_that.address);
      case BridgeShareReach_Refused() when refused != null:
        return refused();
      case BridgeShareReach_Behind() when behind != null:
        return behind();
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeShareReach_Off extends BridgeShareReach {
  const BridgeShareReach_Off() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeShareReach_Off);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareReach.off()';
  }
}

/// @nodoc

class BridgeShareReach_Asking extends BridgeShareReach {
  const BridgeShareReach_Asking() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeShareReach_Asking);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareReach.asking()';
  }
}

/// @nodoc

class BridgeShareReach_Open extends BridgeShareReach {
  const BridgeShareReach_Open({required this.address}) : super._();

  final String address;

  /// Create a copy of BridgeShareReach
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareReach_OpenCopyWith<BridgeShareReach_Open> get copyWith =>
      _$BridgeShareReach_OpenCopyWithImpl<BridgeShareReach_Open>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareReach_Open &&
            (identical(other.address, address) || other.address == address));
  }

  @override
  int get hashCode => Object.hash(runtimeType, address);

  @override
  String toString() {
    return 'BridgeShareReach.open(address: $address)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareReach_OpenCopyWith<$Res>
    implements $BridgeShareReachCopyWith<$Res> {
  factory $BridgeShareReach_OpenCopyWith(BridgeShareReach_Open value,
          $Res Function(BridgeShareReach_Open) _then) =
      _$BridgeShareReach_OpenCopyWithImpl;
  @useResult
  $Res call({String address});
}

/// @nodoc
class _$BridgeShareReach_OpenCopyWithImpl<$Res>
    implements $BridgeShareReach_OpenCopyWith<$Res> {
  _$BridgeShareReach_OpenCopyWithImpl(this._self, this._then);

  final BridgeShareReach_Open _self;
  final $Res Function(BridgeShareReach_Open) _then;

  /// Create a copy of BridgeShareReach
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? address = null,
  }) {
    return _then(BridgeShareReach_Open(
      address: null == address
          ? _self.address
          : address // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeShareReach_Refused extends BridgeShareReach {
  const BridgeShareReach_Refused() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeShareReach_Refused);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareReach.refused()';
  }
}

/// @nodoc

class BridgeShareReach_Behind extends BridgeShareReach {
  const BridgeShareReach_Behind() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeShareReach_Behind);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareReach.behind()';
  }
}

/// @nodoc
mixin _$BridgeShareStarted {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeShareStarted);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareStarted()';
  }
}

/// @nodoc
class $BridgeShareStartedCopyWith<$Res> {
  $BridgeShareStartedCopyWith(
      BridgeShareStarted _, $Res Function(BridgeShareStarted) __);
}

/// Adds pattern-matching-related methods to [BridgeShareStarted].
extension BridgeShareStartedPatterns on BridgeShareStarted {
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
    TResult Function(BridgeShareStarted_Sharing value)? sharing,
    TResult Function(BridgeShareStarted_PortInUse value)? portInUse,
    TResult Function(BridgeShareStarted_Failed value)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareStarted_Sharing() when sharing != null:
        return sharing(_that);
      case BridgeShareStarted_PortInUse() when portInUse != null:
        return portInUse(_that);
      case BridgeShareStarted_Failed() when failed != null:
        return failed(_that);
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
    required TResult Function(BridgeShareStarted_Sharing value) sharing,
    required TResult Function(BridgeShareStarted_PortInUse value) portInUse,
    required TResult Function(BridgeShareStarted_Failed value) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareStarted_Sharing():
        return sharing(_that);
      case BridgeShareStarted_PortInUse():
        return portInUse(_that);
      case BridgeShareStarted_Failed():
        return failed(_that);
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
    TResult? Function(BridgeShareStarted_Sharing value)? sharing,
    TResult? Function(BridgeShareStarted_PortInUse value)? portInUse,
    TResult? Function(BridgeShareStarted_Failed value)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareStarted_Sharing() when sharing != null:
        return sharing(_that);
      case BridgeShareStarted_PortInUse() when portInUse != null:
        return portInUse(_that);
      case BridgeShareStarted_Failed() when failed != null:
        return failed(_that);
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
    TResult Function(int port, String key, int restored)? sharing,
    TResult Function()? portInUse,
    TResult Function()? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareStarted_Sharing() when sharing != null:
        return sharing(_that.port, _that.key, _that.restored);
      case BridgeShareStarted_PortInUse() when portInUse != null:
        return portInUse();
      case BridgeShareStarted_Failed() when failed != null:
        return failed();
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
    required TResult Function(int port, String key, int restored) sharing,
    required TResult Function() portInUse,
    required TResult Function() failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareStarted_Sharing():
        return sharing(_that.port, _that.key, _that.restored);
      case BridgeShareStarted_PortInUse():
        return portInUse();
      case BridgeShareStarted_Failed():
        return failed();
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
    TResult? Function(int port, String key, int restored)? sharing,
    TResult? Function()? portInUse,
    TResult? Function()? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareStarted_Sharing() when sharing != null:
        return sharing(_that.port, _that.key, _that.restored);
      case BridgeShareStarted_PortInUse() when portInUse != null:
        return portInUse();
      case BridgeShareStarted_Failed() when failed != null:
        return failed();
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeShareStarted_Sharing extends BridgeShareStarted {
  const BridgeShareStarted_Sharing(
      {required this.port, required this.key, required this.restored})
      : super._();

  final int port;
  final String key;
  final int restored;

  /// Create a copy of BridgeShareStarted
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareStarted_SharingCopyWith<BridgeShareStarted_Sharing>
      get copyWith =>
          _$BridgeShareStarted_SharingCopyWithImpl<BridgeShareStarted_Sharing>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareStarted_Sharing &&
            (identical(other.port, port) || other.port == port) &&
            (identical(other.key, key) || other.key == key) &&
            (identical(other.restored, restored) ||
                other.restored == restored));
  }

  @override
  int get hashCode => Object.hash(runtimeType, port, key, restored);

  @override
  String toString() {
    return 'BridgeShareStarted.sharing(port: $port, key: $key, restored: $restored)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareStarted_SharingCopyWith<$Res>
    implements $BridgeShareStartedCopyWith<$Res> {
  factory $BridgeShareStarted_SharingCopyWith(BridgeShareStarted_Sharing value,
          $Res Function(BridgeShareStarted_Sharing) _then) =
      _$BridgeShareStarted_SharingCopyWithImpl;
  @useResult
  $Res call({int port, String key, int restored});
}

/// @nodoc
class _$BridgeShareStarted_SharingCopyWithImpl<$Res>
    implements $BridgeShareStarted_SharingCopyWith<$Res> {
  _$BridgeShareStarted_SharingCopyWithImpl(this._self, this._then);

  final BridgeShareStarted_Sharing _self;
  final $Res Function(BridgeShareStarted_Sharing) _then;

  /// Create a copy of BridgeShareStarted
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? port = null,
    Object? key = null,
    Object? restored = null,
  }) {
    return _then(BridgeShareStarted_Sharing(
      port: null == port
          ? _self.port
          : port // ignore: cast_nullable_to_non_nullable
              as int,
      key: null == key
          ? _self.key
          : key // ignore: cast_nullable_to_non_nullable
              as String,
      restored: null == restored
          ? _self.restored
          : restored // ignore: cast_nullable_to_non_nullable
              as int,
    ));
  }
}

/// @nodoc

class BridgeShareStarted_PortInUse extends BridgeShareStarted {
  const BridgeShareStarted_PortInUse() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareStarted_PortInUse);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareStarted.portInUse()';
  }
}

/// @nodoc

class BridgeShareStarted_Failed extends BridgeShareStarted {
  const BridgeShareStarted_Failed() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareStarted_Failed);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareStarted.failed()';
  }
}

// dart format on
