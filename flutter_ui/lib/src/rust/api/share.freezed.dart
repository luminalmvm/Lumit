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
    TResult Function(BridgeJoinOutcome_Versions value)? versions,
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
      case BridgeJoinOutcome_Versions() when versions != null:
        return versions(_that);
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
    required TResult Function(BridgeJoinOutcome_Versions value) versions,
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
      case BridgeJoinOutcome_Versions():
        return versions(_that);
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
    TResult? Function(BridgeJoinOutcome_Versions value)? versions,
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
      case BridgeJoinOutcome_Versions() when versions != null:
        return versions(_that);
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
    TResult Function(BridgeShareVersions versions)? versions,
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
      case BridgeJoinOutcome_Versions() when versions != null:
        return versions(_that.versions);
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
    required TResult Function(BridgeShareVersions versions) versions,
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
      case BridgeJoinOutcome_Versions():
        return versions(_that.versions);
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
    TResult? Function(BridgeShareVersions versions)? versions,
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
      case BridgeJoinOutcome_Versions() when versions != null:
        return versions(_that.versions);
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

class BridgeJoinOutcome_Versions extends BridgeJoinOutcome {
  const BridgeJoinOutcome_Versions({required this.versions}) : super._();

  final BridgeShareVersions versions;

  /// Create a copy of BridgeJoinOutcome
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeJoinOutcome_VersionsCopyWith<BridgeJoinOutcome_Versions>
      get copyWith =>
          _$BridgeJoinOutcome_VersionsCopyWithImpl<BridgeJoinOutcome_Versions>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeJoinOutcome_Versions &&
            (identical(other.versions, versions) ||
                other.versions == versions));
  }

  @override
  int get hashCode => Object.hash(runtimeType, versions);

  @override
  String toString() {
    return 'BridgeJoinOutcome.versions(versions: $versions)';
  }
}

/// @nodoc
abstract mixin class $BridgeJoinOutcome_VersionsCopyWith<$Res>
    implements $BridgeJoinOutcomeCopyWith<$Res> {
  factory $BridgeJoinOutcome_VersionsCopyWith(BridgeJoinOutcome_Versions value,
          $Res Function(BridgeJoinOutcome_Versions) _then) =
      _$BridgeJoinOutcome_VersionsCopyWithImpl;
  @useResult
  $Res call({BridgeShareVersions versions});

  $BridgeShareVersionsCopyWith<$Res> get versions;
}

/// @nodoc
class _$BridgeJoinOutcome_VersionsCopyWithImpl<$Res>
    implements $BridgeJoinOutcome_VersionsCopyWith<$Res> {
  _$BridgeJoinOutcome_VersionsCopyWithImpl(this._self, this._then);

  final BridgeJoinOutcome_Versions _self;
  final $Res Function(BridgeJoinOutcome_Versions) _then;

  /// Create a copy of BridgeJoinOutcome
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? versions = null,
  }) {
    return _then(BridgeJoinOutcome_Versions(
      versions: null == versions
          ? _self.versions
          : versions // ignore: cast_nullable_to_non_nullable
              as BridgeShareVersions,
    ));
  }

  /// Create a copy of BridgeJoinOutcome
  /// with the given fields replaced by the non-null parameter values.
  @override
  @pragma('vm:prefer-inline')
  $BridgeShareVersionsCopyWith<$Res> get versions {
    return $BridgeShareVersionsCopyWith<$Res>(_self.versions, (value) {
      return _then(_self.copyWith(versions: value));
    });
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
mixin _$BridgeOnlineForward {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeOnlineForward);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeOnlineForward()';
  }
}

/// @nodoc
class $BridgeOnlineForwardCopyWith<$Res> {
  $BridgeOnlineForwardCopyWith(
      BridgeOnlineForward _, $Res Function(BridgeOnlineForward) __);
}

/// Adds pattern-matching-related methods to [BridgeOnlineForward].
extension BridgeOnlineForwardPatterns on BridgeOnlineForward {
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
    TResult Function(BridgeOnlineForward_Moved value)? moved,
    TResult Function(BridgeOnlineForward_Unreadable value)? unreadable,
    TResult Function(BridgeOnlineForward_NotHost value)? notHost,
    TResult Function(BridgeOnlineForward_Update value)? update,
    TResult Function(BridgeOnlineForward_Refused value)? refused,
    TResult Function(BridgeOnlineForward_Unreachable value)? unreachable,
    TResult Function(BridgeOnlineForward_Failed value)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOnlineForward_Moved() when moved != null:
        return moved(_that);
      case BridgeOnlineForward_Unreadable() when unreadable != null:
        return unreadable(_that);
      case BridgeOnlineForward_NotHost() when notHost != null:
        return notHost(_that);
      case BridgeOnlineForward_Update() when update != null:
        return update(_that);
      case BridgeOnlineForward_Refused() when refused != null:
        return refused(_that);
      case BridgeOnlineForward_Unreachable() when unreachable != null:
        return unreachable(_that);
      case BridgeOnlineForward_Failed() when failed != null:
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
    required TResult Function(BridgeOnlineForward_Moved value) moved,
    required TResult Function(BridgeOnlineForward_Unreadable value) unreadable,
    required TResult Function(BridgeOnlineForward_NotHost value) notHost,
    required TResult Function(BridgeOnlineForward_Update value) update,
    required TResult Function(BridgeOnlineForward_Refused value) refused,
    required TResult Function(BridgeOnlineForward_Unreachable value)
        unreachable,
    required TResult Function(BridgeOnlineForward_Failed value) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOnlineForward_Moved():
        return moved(_that);
      case BridgeOnlineForward_Unreadable():
        return unreadable(_that);
      case BridgeOnlineForward_NotHost():
        return notHost(_that);
      case BridgeOnlineForward_Update():
        return update(_that);
      case BridgeOnlineForward_Refused():
        return refused(_that);
      case BridgeOnlineForward_Unreachable():
        return unreachable(_that);
      case BridgeOnlineForward_Failed():
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
    TResult? Function(BridgeOnlineForward_Moved value)? moved,
    TResult? Function(BridgeOnlineForward_Unreadable value)? unreadable,
    TResult? Function(BridgeOnlineForward_NotHost value)? notHost,
    TResult? Function(BridgeOnlineForward_Update value)? update,
    TResult? Function(BridgeOnlineForward_Refused value)? refused,
    TResult? Function(BridgeOnlineForward_Unreachable value)? unreachable,
    TResult? Function(BridgeOnlineForward_Failed value)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOnlineForward_Moved() when moved != null:
        return moved(_that);
      case BridgeOnlineForward_Unreadable() when unreadable != null:
        return unreadable(_that);
      case BridgeOnlineForward_NotHost() when notHost != null:
        return notHost(_that);
      case BridgeOnlineForward_Update() when update != null:
        return update(_that);
      case BridgeOnlineForward_Refused() when refused != null:
        return refused(_that);
      case BridgeOnlineForward_Unreachable() when unreachable != null:
        return unreachable(_that);
      case BridgeOnlineForward_Failed() when failed != null:
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
    TResult Function()? moved,
    TResult Function()? unreadable,
    TResult Function()? notHost,
    TResult Function(String version)? update,
    TResult Function(String code)? refused,
    TResult Function()? unreachable,
    TResult Function()? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOnlineForward_Moved() when moved != null:
        return moved();
      case BridgeOnlineForward_Unreadable() when unreadable != null:
        return unreadable();
      case BridgeOnlineForward_NotHost() when notHost != null:
        return notHost();
      case BridgeOnlineForward_Update() when update != null:
        return update(_that.version);
      case BridgeOnlineForward_Refused() when refused != null:
        return refused(_that.code);
      case BridgeOnlineForward_Unreachable() when unreachable != null:
        return unreachable();
      case BridgeOnlineForward_Failed() when failed != null:
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
    required TResult Function() moved,
    required TResult Function() unreadable,
    required TResult Function() notHost,
    required TResult Function(String version) update,
    required TResult Function(String code) refused,
    required TResult Function() unreachable,
    required TResult Function() failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOnlineForward_Moved():
        return moved();
      case BridgeOnlineForward_Unreadable():
        return unreadable();
      case BridgeOnlineForward_NotHost():
        return notHost();
      case BridgeOnlineForward_Update():
        return update(_that.version);
      case BridgeOnlineForward_Refused():
        return refused(_that.code);
      case BridgeOnlineForward_Unreachable():
        return unreachable();
      case BridgeOnlineForward_Failed():
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
    TResult? Function()? moved,
    TResult? Function()? unreadable,
    TResult? Function()? notHost,
    TResult? Function(String version)? update,
    TResult? Function(String code)? refused,
    TResult? Function()? unreachable,
    TResult? Function()? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOnlineForward_Moved() when moved != null:
        return moved();
      case BridgeOnlineForward_Unreadable() when unreadable != null:
        return unreadable();
      case BridgeOnlineForward_NotHost() when notHost != null:
        return notHost();
      case BridgeOnlineForward_Update() when update != null:
        return update(_that.version);
      case BridgeOnlineForward_Refused() when refused != null:
        return refused(_that.code);
      case BridgeOnlineForward_Unreachable() when unreachable != null:
        return unreachable();
      case BridgeOnlineForward_Failed() when failed != null:
        return failed();
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeOnlineForward_Moved extends BridgeOnlineForward {
  const BridgeOnlineForward_Moved() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeOnlineForward_Moved);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeOnlineForward.moved()';
  }
}

/// @nodoc

class BridgeOnlineForward_Unreadable extends BridgeOnlineForward {
  const BridgeOnlineForward_Unreadable() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeOnlineForward_Unreadable);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeOnlineForward.unreadable()';
  }
}

/// @nodoc

class BridgeOnlineForward_NotHost extends BridgeOnlineForward {
  const BridgeOnlineForward_NotHost() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeOnlineForward_NotHost);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeOnlineForward.notHost()';
  }
}

/// @nodoc

class BridgeOnlineForward_Update extends BridgeOnlineForward {
  const BridgeOnlineForward_Update({required this.version}) : super._();

  final String version;

  /// Create a copy of BridgeOnlineForward
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeOnlineForward_UpdateCopyWith<BridgeOnlineForward_Update>
      get copyWith =>
          _$BridgeOnlineForward_UpdateCopyWithImpl<BridgeOnlineForward_Update>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeOnlineForward_Update &&
            (identical(other.version, version) || other.version == version));
  }

  @override
  int get hashCode => Object.hash(runtimeType, version);

  @override
  String toString() {
    return 'BridgeOnlineForward.update(version: $version)';
  }
}

/// @nodoc
abstract mixin class $BridgeOnlineForward_UpdateCopyWith<$Res>
    implements $BridgeOnlineForwardCopyWith<$Res> {
  factory $BridgeOnlineForward_UpdateCopyWith(BridgeOnlineForward_Update value,
          $Res Function(BridgeOnlineForward_Update) _then) =
      _$BridgeOnlineForward_UpdateCopyWithImpl;
  @useResult
  $Res call({String version});
}

/// @nodoc
class _$BridgeOnlineForward_UpdateCopyWithImpl<$Res>
    implements $BridgeOnlineForward_UpdateCopyWith<$Res> {
  _$BridgeOnlineForward_UpdateCopyWithImpl(this._self, this._then);

  final BridgeOnlineForward_Update _self;
  final $Res Function(BridgeOnlineForward_Update) _then;

  /// Create a copy of BridgeOnlineForward
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? version = null,
  }) {
    return _then(BridgeOnlineForward_Update(
      version: null == version
          ? _self.version
          : version // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeOnlineForward_Refused extends BridgeOnlineForward {
  const BridgeOnlineForward_Refused({required this.code}) : super._();

  final String code;

  /// Create a copy of BridgeOnlineForward
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeOnlineForward_RefusedCopyWith<BridgeOnlineForward_Refused>
      get copyWith => _$BridgeOnlineForward_RefusedCopyWithImpl<
          BridgeOnlineForward_Refused>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeOnlineForward_Refused &&
            (identical(other.code, code) || other.code == code));
  }

  @override
  int get hashCode => Object.hash(runtimeType, code);

  @override
  String toString() {
    return 'BridgeOnlineForward.refused(code: $code)';
  }
}

/// @nodoc
abstract mixin class $BridgeOnlineForward_RefusedCopyWith<$Res>
    implements $BridgeOnlineForwardCopyWith<$Res> {
  factory $BridgeOnlineForward_RefusedCopyWith(
          BridgeOnlineForward_Refused value,
          $Res Function(BridgeOnlineForward_Refused) _then) =
      _$BridgeOnlineForward_RefusedCopyWithImpl;
  @useResult
  $Res call({String code});
}

/// @nodoc
class _$BridgeOnlineForward_RefusedCopyWithImpl<$Res>
    implements $BridgeOnlineForward_RefusedCopyWith<$Res> {
  _$BridgeOnlineForward_RefusedCopyWithImpl(this._self, this._then);

  final BridgeOnlineForward_Refused _self;
  final $Res Function(BridgeOnlineForward_Refused) _then;

  /// Create a copy of BridgeOnlineForward
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? code = null,
  }) {
    return _then(BridgeOnlineForward_Refused(
      code: null == code
          ? _self.code
          : code // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeOnlineForward_Unreachable extends BridgeOnlineForward {
  const BridgeOnlineForward_Unreachable() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeOnlineForward_Unreachable);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeOnlineForward.unreachable()';
  }
}

/// @nodoc

class BridgeOnlineForward_Failed extends BridgeOnlineForward {
  const BridgeOnlineForward_Failed() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeOnlineForward_Failed);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeOnlineForward.failed()';
  }
}

/// @nodoc
mixin _$BridgeOnlineStarted {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeOnlineStarted);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeOnlineStarted()';
  }
}

/// @nodoc
class $BridgeOnlineStartedCopyWith<$Res> {
  $BridgeOnlineStartedCopyWith(
      BridgeOnlineStarted _, $Res Function(BridgeOnlineStarted) __);
}

/// Adds pattern-matching-related methods to [BridgeOnlineStarted].
extension BridgeOnlineStartedPatterns on BridgeOnlineStarted {
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
    TResult Function(BridgeOnlineStarted_Online value)? online,
    TResult Function(BridgeOnlineStarted_Refused value)? refused,
    TResult Function(BridgeOnlineStarted_Unreachable value)? unreachable,
    TResult Function(BridgeOnlineStarted_Failed value)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOnlineStarted_Online() when online != null:
        return online(_that);
      case BridgeOnlineStarted_Refused() when refused != null:
        return refused(_that);
      case BridgeOnlineStarted_Unreachable() when unreachable != null:
        return unreachable(_that);
      case BridgeOnlineStarted_Failed() when failed != null:
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
    required TResult Function(BridgeOnlineStarted_Online value) online,
    required TResult Function(BridgeOnlineStarted_Refused value) refused,
    required TResult Function(BridgeOnlineStarted_Unreachable value)
        unreachable,
    required TResult Function(BridgeOnlineStarted_Failed value) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOnlineStarted_Online():
        return online(_that);
      case BridgeOnlineStarted_Refused():
        return refused(_that);
      case BridgeOnlineStarted_Unreachable():
        return unreachable(_that);
      case BridgeOnlineStarted_Failed():
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
    TResult? Function(BridgeOnlineStarted_Online value)? online,
    TResult? Function(BridgeOnlineStarted_Refused value)? refused,
    TResult? Function(BridgeOnlineStarted_Unreachable value)? unreachable,
    TResult? Function(BridgeOnlineStarted_Failed value)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOnlineStarted_Online() when online != null:
        return online(_that);
      case BridgeOnlineStarted_Refused() when refused != null:
        return refused(_that);
      case BridgeOnlineStarted_Unreachable() when unreachable != null:
        return unreachable(_that);
      case BridgeOnlineStarted_Failed() when failed != null:
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
    TResult Function(String link)? online,
    TResult Function(String code)? refused,
    TResult Function()? unreachable,
    TResult Function()? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOnlineStarted_Online() when online != null:
        return online(_that.link);
      case BridgeOnlineStarted_Refused() when refused != null:
        return refused(_that.code);
      case BridgeOnlineStarted_Unreachable() when unreachable != null:
        return unreachable();
      case BridgeOnlineStarted_Failed() when failed != null:
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
    required TResult Function(String link) online,
    required TResult Function(String code) refused,
    required TResult Function() unreachable,
    required TResult Function() failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOnlineStarted_Online():
        return online(_that.link);
      case BridgeOnlineStarted_Refused():
        return refused(_that.code);
      case BridgeOnlineStarted_Unreachable():
        return unreachable();
      case BridgeOnlineStarted_Failed():
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
    TResult? Function(String link)? online,
    TResult? Function(String code)? refused,
    TResult? Function()? unreachable,
    TResult? Function()? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeOnlineStarted_Online() when online != null:
        return online(_that.link);
      case BridgeOnlineStarted_Refused() when refused != null:
        return refused(_that.code);
      case BridgeOnlineStarted_Unreachable() when unreachable != null:
        return unreachable();
      case BridgeOnlineStarted_Failed() when failed != null:
        return failed();
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeOnlineStarted_Online extends BridgeOnlineStarted {
  const BridgeOnlineStarted_Online({required this.link}) : super._();

  final String link;

  /// Create a copy of BridgeOnlineStarted
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeOnlineStarted_OnlineCopyWith<BridgeOnlineStarted_Online>
      get copyWith =>
          _$BridgeOnlineStarted_OnlineCopyWithImpl<BridgeOnlineStarted_Online>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeOnlineStarted_Online &&
            (identical(other.link, link) || other.link == link));
  }

  @override
  int get hashCode => Object.hash(runtimeType, link);

  @override
  String toString() {
    return 'BridgeOnlineStarted.online(link: $link)';
  }
}

/// @nodoc
abstract mixin class $BridgeOnlineStarted_OnlineCopyWith<$Res>
    implements $BridgeOnlineStartedCopyWith<$Res> {
  factory $BridgeOnlineStarted_OnlineCopyWith(BridgeOnlineStarted_Online value,
          $Res Function(BridgeOnlineStarted_Online) _then) =
      _$BridgeOnlineStarted_OnlineCopyWithImpl;
  @useResult
  $Res call({String link});
}

/// @nodoc
class _$BridgeOnlineStarted_OnlineCopyWithImpl<$Res>
    implements $BridgeOnlineStarted_OnlineCopyWith<$Res> {
  _$BridgeOnlineStarted_OnlineCopyWithImpl(this._self, this._then);

  final BridgeOnlineStarted_Online _self;
  final $Res Function(BridgeOnlineStarted_Online) _then;

  /// Create a copy of BridgeOnlineStarted
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? link = null,
  }) {
    return _then(BridgeOnlineStarted_Online(
      link: null == link
          ? _self.link
          : link // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeOnlineStarted_Refused extends BridgeOnlineStarted {
  const BridgeOnlineStarted_Refused({required this.code}) : super._();

  final String code;

  /// Create a copy of BridgeOnlineStarted
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeOnlineStarted_RefusedCopyWith<BridgeOnlineStarted_Refused>
      get copyWith => _$BridgeOnlineStarted_RefusedCopyWithImpl<
          BridgeOnlineStarted_Refused>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeOnlineStarted_Refused &&
            (identical(other.code, code) || other.code == code));
  }

  @override
  int get hashCode => Object.hash(runtimeType, code);

  @override
  String toString() {
    return 'BridgeOnlineStarted.refused(code: $code)';
  }
}

/// @nodoc
abstract mixin class $BridgeOnlineStarted_RefusedCopyWith<$Res>
    implements $BridgeOnlineStartedCopyWith<$Res> {
  factory $BridgeOnlineStarted_RefusedCopyWith(
          BridgeOnlineStarted_Refused value,
          $Res Function(BridgeOnlineStarted_Refused) _then) =
      _$BridgeOnlineStarted_RefusedCopyWithImpl;
  @useResult
  $Res call({String code});
}

/// @nodoc
class _$BridgeOnlineStarted_RefusedCopyWithImpl<$Res>
    implements $BridgeOnlineStarted_RefusedCopyWith<$Res> {
  _$BridgeOnlineStarted_RefusedCopyWithImpl(this._self, this._then);

  final BridgeOnlineStarted_Refused _self;
  final $Res Function(BridgeOnlineStarted_Refused) _then;

  /// Create a copy of BridgeOnlineStarted
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? code = null,
  }) {
    return _then(BridgeOnlineStarted_Refused(
      code: null == code
          ? _self.code
          : code // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeOnlineStarted_Unreachable extends BridgeOnlineStarted {
  const BridgeOnlineStarted_Unreachable() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeOnlineStarted_Unreachable);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeOnlineStarted.unreachable()';
  }
}

/// @nodoc

class BridgeOnlineStarted_Failed extends BridgeOnlineStarted {
  const BridgeOnlineStarted_Failed() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeOnlineStarted_Failed);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeOnlineStarted.failed()';
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
    TResult Function(BridgeShareEvent_Replaced value)? replaced,
    TResult Function(BridgeShareEvent_Ended value)? ended,
    TResult Function(BridgeShareEvent_Reach value)? reach,
    TResult Function(BridgeShareEvent_Relayed value)? relayed,
    TResult Function(BridgeShareEvent_Footage value)? footage,
    TResult Function(BridgeShareEvent_ExportAsked value)? exportAsked,
    TResult Function(BridgeShareEvent_Writable value)? writable,
    TResult Function(BridgeShareEvent_Activity value)? activity,
    TResult Function(BridgeShareEvent_Versions value)? versions,
    TResult Function(BridgeShareEvent_LookOnly value)? lookOnly,
    TResult Function(BridgeShareEvent_LookInvite value)? lookInvite,
    TResult Function(BridgeShareEvent_TooLarge value)? tooLarge,
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
      case BridgeShareEvent_Replaced() when replaced != null:
        return replaced(_that);
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
      case BridgeShareEvent_Writable() when writable != null:
        return writable(_that);
      case BridgeShareEvent_Activity() when activity != null:
        return activity(_that);
      case BridgeShareEvent_Versions() when versions != null:
        return versions(_that);
      case BridgeShareEvent_LookOnly() when lookOnly != null:
        return lookOnly(_that);
      case BridgeShareEvent_LookInvite() when lookInvite != null:
        return lookInvite(_that);
      case BridgeShareEvent_TooLarge() when tooLarge != null:
        return tooLarge(_that);
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
    required TResult Function(BridgeShareEvent_Replaced value) replaced,
    required TResult Function(BridgeShareEvent_Ended value) ended,
    required TResult Function(BridgeShareEvent_Reach value) reach,
    required TResult Function(BridgeShareEvent_Relayed value) relayed,
    required TResult Function(BridgeShareEvent_Footage value) footage,
    required TResult Function(BridgeShareEvent_ExportAsked value) exportAsked,
    required TResult Function(BridgeShareEvent_Writable value) writable,
    required TResult Function(BridgeShareEvent_Activity value) activity,
    required TResult Function(BridgeShareEvent_Versions value) versions,
    required TResult Function(BridgeShareEvent_LookOnly value) lookOnly,
    required TResult Function(BridgeShareEvent_LookInvite value) lookInvite,
    required TResult Function(BridgeShareEvent_TooLarge value) tooLarge,
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
      case BridgeShareEvent_Replaced():
        return replaced(_that);
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
      case BridgeShareEvent_Writable():
        return writable(_that);
      case BridgeShareEvent_Activity():
        return activity(_that);
      case BridgeShareEvent_Versions():
        return versions(_that);
      case BridgeShareEvent_LookOnly():
        return lookOnly(_that);
      case BridgeShareEvent_LookInvite():
        return lookInvite(_that);
      case BridgeShareEvent_TooLarge():
        return tooLarge(_that);
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
    TResult? Function(BridgeShareEvent_Replaced value)? replaced,
    TResult? Function(BridgeShareEvent_Ended value)? ended,
    TResult? Function(BridgeShareEvent_Reach value)? reach,
    TResult? Function(BridgeShareEvent_Relayed value)? relayed,
    TResult? Function(BridgeShareEvent_Footage value)? footage,
    TResult? Function(BridgeShareEvent_ExportAsked value)? exportAsked,
    TResult? Function(BridgeShareEvent_Writable value)? writable,
    TResult? Function(BridgeShareEvent_Activity value)? activity,
    TResult? Function(BridgeShareEvent_Versions value)? versions,
    TResult? Function(BridgeShareEvent_LookOnly value)? lookOnly,
    TResult? Function(BridgeShareEvent_LookInvite value)? lookInvite,
    TResult? Function(BridgeShareEvent_TooLarge value)? tooLarge,
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
      case BridgeShareEvent_Replaced() when replaced != null:
        return replaced(_that);
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
      case BridgeShareEvent_Writable() when writable != null:
        return writable(_that);
      case BridgeShareEvent_Activity() when activity != null:
        return activity(_that);
      case BridgeShareEvent_Versions() when versions != null:
        return versions(_that);
      case BridgeShareEvent_LookOnly() when lookOnly != null:
        return lookOnly(_that);
      case BridgeShareEvent_LookInvite() when lookInvite != null:
        return lookInvite(_that);
      case BridgeShareEvent_TooLarge() when tooLarge != null:
        return tooLarge(_that);
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
    TResult Function(int held, int refused, List<BridgeShareDropped> dropped)?
        back,
    TResult Function()? elsewhere,
    TResult Function()? replaced,
    TResult Function(BridgeShareEnding reason)? ended,
    TResult Function(BridgeShareReach reach)? reach,
    TResult Function(BridgeShareRelayed relayed)? relayed,
    TResult Function(bool placed)? footage,
    TResult Function(String job, int from, String comp)? exportAsked,
    TResult Function(bool writable)? writable,
    TResult Function()? activity,
    TResult Function(BridgeShareVersions versions)? versions,
    TResult Function()? lookOnly,
    TResult Function(bool lookOn)? lookInvite,
    TResult Function(bool tooLarge)? tooLarge,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareEvent_People() when people != null:
        return people(_that.people);
      case BridgeShareEvent_Away() when away != null:
        return away();
      case BridgeShareEvent_Back() when back != null:
        return back(_that.held, _that.refused, _that.dropped);
      case BridgeShareEvent_Elsewhere() when elsewhere != null:
        return elsewhere();
      case BridgeShareEvent_Replaced() when replaced != null:
        return replaced();
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
      case BridgeShareEvent_Writable() when writable != null:
        return writable(_that.writable);
      case BridgeShareEvent_Activity() when activity != null:
        return activity();
      case BridgeShareEvent_Versions() when versions != null:
        return versions(_that.versions);
      case BridgeShareEvent_LookOnly() when lookOnly != null:
        return lookOnly();
      case BridgeShareEvent_LookInvite() when lookInvite != null:
        return lookInvite(_that.lookOn);
      case BridgeShareEvent_TooLarge() when tooLarge != null:
        return tooLarge(_that.tooLarge);
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
    required TResult Function(
            int held, int refused, List<BridgeShareDropped> dropped)
        back,
    required TResult Function() elsewhere,
    required TResult Function() replaced,
    required TResult Function(BridgeShareEnding reason) ended,
    required TResult Function(BridgeShareReach reach) reach,
    required TResult Function(BridgeShareRelayed relayed) relayed,
    required TResult Function(bool placed) footage,
    required TResult Function(String job, int from, String comp) exportAsked,
    required TResult Function(bool writable) writable,
    required TResult Function() activity,
    required TResult Function(BridgeShareVersions versions) versions,
    required TResult Function() lookOnly,
    required TResult Function(bool lookOn) lookInvite,
    required TResult Function(bool tooLarge) tooLarge,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareEvent_People():
        return people(_that.people);
      case BridgeShareEvent_Away():
        return away();
      case BridgeShareEvent_Back():
        return back(_that.held, _that.refused, _that.dropped);
      case BridgeShareEvent_Elsewhere():
        return elsewhere();
      case BridgeShareEvent_Replaced():
        return replaced();
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
      case BridgeShareEvent_Writable():
        return writable(_that.writable);
      case BridgeShareEvent_Activity():
        return activity();
      case BridgeShareEvent_Versions():
        return versions(_that.versions);
      case BridgeShareEvent_LookOnly():
        return lookOnly();
      case BridgeShareEvent_LookInvite():
        return lookInvite(_that.lookOn);
      case BridgeShareEvent_TooLarge():
        return tooLarge(_that.tooLarge);
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
    TResult? Function(int held, int refused, List<BridgeShareDropped> dropped)?
        back,
    TResult? Function()? elsewhere,
    TResult? Function()? replaced,
    TResult? Function(BridgeShareEnding reason)? ended,
    TResult? Function(BridgeShareReach reach)? reach,
    TResult? Function(BridgeShareRelayed relayed)? relayed,
    TResult? Function(bool placed)? footage,
    TResult? Function(String job, int from, String comp)? exportAsked,
    TResult? Function(bool writable)? writable,
    TResult? Function()? activity,
    TResult? Function(BridgeShareVersions versions)? versions,
    TResult? Function()? lookOnly,
    TResult? Function(bool lookOn)? lookInvite,
    TResult? Function(bool tooLarge)? tooLarge,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareEvent_People() when people != null:
        return people(_that.people);
      case BridgeShareEvent_Away() when away != null:
        return away();
      case BridgeShareEvent_Back() when back != null:
        return back(_that.held, _that.refused, _that.dropped);
      case BridgeShareEvent_Elsewhere() when elsewhere != null:
        return elsewhere();
      case BridgeShareEvent_Replaced() when replaced != null:
        return replaced();
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
      case BridgeShareEvent_Writable() when writable != null:
        return writable(_that.writable);
      case BridgeShareEvent_Activity() when activity != null:
        return activity();
      case BridgeShareEvent_Versions() when versions != null:
        return versions(_that.versions);
      case BridgeShareEvent_LookOnly() when lookOnly != null:
        return lookOnly();
      case BridgeShareEvent_LookInvite() when lookInvite != null:
        return lookInvite(_that.lookOn);
      case BridgeShareEvent_TooLarge() when tooLarge != null:
        return tooLarge(_that.tooLarge);
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
  const BridgeShareEvent_Back(
      {required this.held,
      required this.refused,
      required final List<BridgeShareDropped> dropped})
      : _dropped = dropped,
        super._();

  final int held;
  final int refused;
  final List<BridgeShareDropped> _dropped;
  List<BridgeShareDropped> get dropped {
    if (_dropped is EqualUnmodifiableListView) return _dropped;
    // ignore: implicit_dynamic_type
    return EqualUnmodifiableListView(_dropped);
  }

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
            (identical(other.refused, refused) || other.refused == refused) &&
            const DeepCollectionEquality().equals(other._dropped, _dropped));
  }

  @override
  int get hashCode => Object.hash(runtimeType, held, refused,
      const DeepCollectionEquality().hash(_dropped));

  @override
  String toString() {
    return 'BridgeShareEvent.back(held: $held, refused: $refused, dropped: $dropped)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareEvent_BackCopyWith<$Res>
    implements $BridgeShareEventCopyWith<$Res> {
  factory $BridgeShareEvent_BackCopyWith(BridgeShareEvent_Back value,
          $Res Function(BridgeShareEvent_Back) _then) =
      _$BridgeShareEvent_BackCopyWithImpl;
  @useResult
  $Res call({int held, int refused, List<BridgeShareDropped> dropped});
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
    Object? dropped = null,
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
      dropped: null == dropped
          ? _self._dropped
          : dropped // ignore: cast_nullable_to_non_nullable
              as List<BridgeShareDropped>,
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

class BridgeShareEvent_Replaced extends BridgeShareEvent {
  const BridgeShareEvent_Replaced() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEvent_Replaced);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareEvent.replaced()';
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

class BridgeShareEvent_Writable extends BridgeShareEvent {
  const BridgeShareEvent_Writable({required this.writable}) : super._();

  final bool writable;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareEvent_WritableCopyWith<BridgeShareEvent_Writable> get copyWith =>
      _$BridgeShareEvent_WritableCopyWithImpl<BridgeShareEvent_Writable>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEvent_Writable &&
            (identical(other.writable, writable) ||
                other.writable == writable));
  }

  @override
  int get hashCode => Object.hash(runtimeType, writable);

  @override
  String toString() {
    return 'BridgeShareEvent.writable(writable: $writable)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareEvent_WritableCopyWith<$Res>
    implements $BridgeShareEventCopyWith<$Res> {
  factory $BridgeShareEvent_WritableCopyWith(BridgeShareEvent_Writable value,
          $Res Function(BridgeShareEvent_Writable) _then) =
      _$BridgeShareEvent_WritableCopyWithImpl;
  @useResult
  $Res call({bool writable});
}

/// @nodoc
class _$BridgeShareEvent_WritableCopyWithImpl<$Res>
    implements $BridgeShareEvent_WritableCopyWith<$Res> {
  _$BridgeShareEvent_WritableCopyWithImpl(this._self, this._then);

  final BridgeShareEvent_Writable _self;
  final $Res Function(BridgeShareEvent_Writable) _then;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? writable = null,
  }) {
    return _then(BridgeShareEvent_Writable(
      writable: null == writable
          ? _self.writable
          : writable // ignore: cast_nullable_to_non_nullable
              as bool,
    ));
  }
}

/// @nodoc

class BridgeShareEvent_Activity extends BridgeShareEvent {
  const BridgeShareEvent_Activity() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEvent_Activity);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareEvent.activity()';
  }
}

/// @nodoc

class BridgeShareEvent_Versions extends BridgeShareEvent {
  const BridgeShareEvent_Versions({required this.versions}) : super._();

  final BridgeShareVersions versions;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareEvent_VersionsCopyWith<BridgeShareEvent_Versions> get copyWith =>
      _$BridgeShareEvent_VersionsCopyWithImpl<BridgeShareEvent_Versions>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEvent_Versions &&
            (identical(other.versions, versions) ||
                other.versions == versions));
  }

  @override
  int get hashCode => Object.hash(runtimeType, versions);

  @override
  String toString() {
    return 'BridgeShareEvent.versions(versions: $versions)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareEvent_VersionsCopyWith<$Res>
    implements $BridgeShareEventCopyWith<$Res> {
  factory $BridgeShareEvent_VersionsCopyWith(BridgeShareEvent_Versions value,
          $Res Function(BridgeShareEvent_Versions) _then) =
      _$BridgeShareEvent_VersionsCopyWithImpl;
  @useResult
  $Res call({BridgeShareVersions versions});

  $BridgeShareVersionsCopyWith<$Res> get versions;
}

/// @nodoc
class _$BridgeShareEvent_VersionsCopyWithImpl<$Res>
    implements $BridgeShareEvent_VersionsCopyWith<$Res> {
  _$BridgeShareEvent_VersionsCopyWithImpl(this._self, this._then);

  final BridgeShareEvent_Versions _self;
  final $Res Function(BridgeShareEvent_Versions) _then;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? versions = null,
  }) {
    return _then(BridgeShareEvent_Versions(
      versions: null == versions
          ? _self.versions
          : versions // ignore: cast_nullable_to_non_nullable
              as BridgeShareVersions,
    ));
  }

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @override
  @pragma('vm:prefer-inline')
  $BridgeShareVersionsCopyWith<$Res> get versions {
    return $BridgeShareVersionsCopyWith<$Res>(_self.versions, (value) {
      return _then(_self.copyWith(versions: value));
    });
  }
}

/// @nodoc

class BridgeShareEvent_LookOnly extends BridgeShareEvent {
  const BridgeShareEvent_LookOnly() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEvent_LookOnly);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeShareEvent.lookOnly()';
  }
}

/// @nodoc

class BridgeShareEvent_LookInvite extends BridgeShareEvent {
  const BridgeShareEvent_LookInvite({required this.lookOn}) : super._();

  final bool lookOn;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareEvent_LookInviteCopyWith<BridgeShareEvent_LookInvite>
      get copyWith => _$BridgeShareEvent_LookInviteCopyWithImpl<
          BridgeShareEvent_LookInvite>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEvent_LookInvite &&
            (identical(other.lookOn, lookOn) || other.lookOn == lookOn));
  }

  @override
  int get hashCode => Object.hash(runtimeType, lookOn);

  @override
  String toString() {
    return 'BridgeShareEvent.lookInvite(lookOn: $lookOn)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareEvent_LookInviteCopyWith<$Res>
    implements $BridgeShareEventCopyWith<$Res> {
  factory $BridgeShareEvent_LookInviteCopyWith(
          BridgeShareEvent_LookInvite value,
          $Res Function(BridgeShareEvent_LookInvite) _then) =
      _$BridgeShareEvent_LookInviteCopyWithImpl;
  @useResult
  $Res call({bool lookOn});
}

/// @nodoc
class _$BridgeShareEvent_LookInviteCopyWithImpl<$Res>
    implements $BridgeShareEvent_LookInviteCopyWith<$Res> {
  _$BridgeShareEvent_LookInviteCopyWithImpl(this._self, this._then);

  final BridgeShareEvent_LookInvite _self;
  final $Res Function(BridgeShareEvent_LookInvite) _then;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? lookOn = null,
  }) {
    return _then(BridgeShareEvent_LookInvite(
      lookOn: null == lookOn
          ? _self.lookOn
          : lookOn // ignore: cast_nullable_to_non_nullable
              as bool,
    ));
  }
}

/// @nodoc

class BridgeShareEvent_TooLarge extends BridgeShareEvent {
  const BridgeShareEvent_TooLarge({required this.tooLarge}) : super._();

  final bool tooLarge;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareEvent_TooLargeCopyWith<BridgeShareEvent_TooLarge> get copyWith =>
      _$BridgeShareEvent_TooLargeCopyWithImpl<BridgeShareEvent_TooLarge>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareEvent_TooLarge &&
            (identical(other.tooLarge, tooLarge) ||
                other.tooLarge == tooLarge));
  }

  @override
  int get hashCode => Object.hash(runtimeType, tooLarge);

  @override
  String toString() {
    return 'BridgeShareEvent.tooLarge(tooLarge: $tooLarge)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareEvent_TooLargeCopyWith<$Res>
    implements $BridgeShareEventCopyWith<$Res> {
  factory $BridgeShareEvent_TooLargeCopyWith(BridgeShareEvent_TooLarge value,
          $Res Function(BridgeShareEvent_TooLarge) _then) =
      _$BridgeShareEvent_TooLargeCopyWithImpl;
  @useResult
  $Res call({bool tooLarge});
}

/// @nodoc
class _$BridgeShareEvent_TooLargeCopyWithImpl<$Res>
    implements $BridgeShareEvent_TooLargeCopyWith<$Res> {
  _$BridgeShareEvent_TooLargeCopyWithImpl(this._self, this._then);

  final BridgeShareEvent_TooLarge _self;
  final $Res Function(BridgeShareEvent_TooLarge) _then;

  /// Create a copy of BridgeShareEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? tooLarge = null,
  }) {
    return _then(BridgeShareEvent_TooLarge(
      tooLarge: null == tooLarge
          ? _self.tooLarge
          : tooLarge // ignore: cast_nullable_to_non_nullable
              as bool,
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

/// @nodoc
mixin _$BridgeShareVersions {
  String get version;

  /// Create a copy of BridgeShareVersions
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareVersionsCopyWith<BridgeShareVersions> get copyWith =>
      _$BridgeShareVersionsCopyWithImpl<BridgeShareVersions>(
          this as BridgeShareVersions, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareVersions &&
            (identical(other.version, version) || other.version == version));
  }

  @override
  int get hashCode => Object.hash(runtimeType, version);

  @override
  String toString() {
    return 'BridgeShareVersions(version: $version)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareVersionsCopyWith<$Res> {
  factory $BridgeShareVersionsCopyWith(
          BridgeShareVersions value, $Res Function(BridgeShareVersions) _then) =
      _$BridgeShareVersionsCopyWithImpl;
  @useResult
  $Res call({String version});
}

/// @nodoc
class _$BridgeShareVersionsCopyWithImpl<$Res>
    implements $BridgeShareVersionsCopyWith<$Res> {
  _$BridgeShareVersionsCopyWithImpl(this._self, this._then);

  final BridgeShareVersions _self;
  final $Res Function(BridgeShareVersions) _then;

  /// Create a copy of BridgeShareVersions
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  @override
  $Res call({
    Object? version = null,
  }) {
    return _then(_self.copyWith(
      version: null == version
          ? _self.version
          : version // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// Adds pattern-matching-related methods to [BridgeShareVersions].
extension BridgeShareVersionsPatterns on BridgeShareVersions {
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
    TResult Function(BridgeShareVersions_Update value)? update,
    TResult Function(BridgeShareVersions_Movable value)? movable,
    TResult Function(BridgeShareVersions_HostFirst value)? hostFirst,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareVersions_Update() when update != null:
        return update(_that);
      case BridgeShareVersions_Movable() when movable != null:
        return movable(_that);
      case BridgeShareVersions_HostFirst() when hostFirst != null:
        return hostFirst(_that);
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
    required TResult Function(BridgeShareVersions_Update value) update,
    required TResult Function(BridgeShareVersions_Movable value) movable,
    required TResult Function(BridgeShareVersions_HostFirst value) hostFirst,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareVersions_Update():
        return update(_that);
      case BridgeShareVersions_Movable():
        return movable(_that);
      case BridgeShareVersions_HostFirst():
        return hostFirst(_that);
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
    TResult? Function(BridgeShareVersions_Update value)? update,
    TResult? Function(BridgeShareVersions_Movable value)? movable,
    TResult? Function(BridgeShareVersions_HostFirst value)? hostFirst,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareVersions_Update() when update != null:
        return update(_that);
      case BridgeShareVersions_Movable() when movable != null:
        return movable(_that);
      case BridgeShareVersions_HostFirst() when hostFirst != null:
        return hostFirst(_that);
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
    TResult Function(String version)? update,
    TResult Function(String version)? movable,
    TResult Function(String version)? hostFirst,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareVersions_Update() when update != null:
        return update(_that.version);
      case BridgeShareVersions_Movable() when movable != null:
        return movable(_that.version);
      case BridgeShareVersions_HostFirst() when hostFirst != null:
        return hostFirst(_that.version);
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
    required TResult Function(String version) update,
    required TResult Function(String version) movable,
    required TResult Function(String version) hostFirst,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareVersions_Update():
        return update(_that.version);
      case BridgeShareVersions_Movable():
        return movable(_that.version);
      case BridgeShareVersions_HostFirst():
        return hostFirst(_that.version);
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
    TResult? Function(String version)? update,
    TResult? Function(String version)? movable,
    TResult? Function(String version)? hostFirst,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeShareVersions_Update() when update != null:
        return update(_that.version);
      case BridgeShareVersions_Movable() when movable != null:
        return movable(_that.version);
      case BridgeShareVersions_HostFirst() when hostFirst != null:
        return hostFirst(_that.version);
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeShareVersions_Update extends BridgeShareVersions {
  const BridgeShareVersions_Update({required this.version}) : super._();

  @override
  final String version;

  /// Create a copy of BridgeShareVersions
  /// with the given fields replaced by the non-null parameter values.
  @override
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareVersions_UpdateCopyWith<BridgeShareVersions_Update>
      get copyWith =>
          _$BridgeShareVersions_UpdateCopyWithImpl<BridgeShareVersions_Update>(
              this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareVersions_Update &&
            (identical(other.version, version) || other.version == version));
  }

  @override
  int get hashCode => Object.hash(runtimeType, version);

  @override
  String toString() {
    return 'BridgeShareVersions.update(version: $version)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareVersions_UpdateCopyWith<$Res>
    implements $BridgeShareVersionsCopyWith<$Res> {
  factory $BridgeShareVersions_UpdateCopyWith(BridgeShareVersions_Update value,
          $Res Function(BridgeShareVersions_Update) _then) =
      _$BridgeShareVersions_UpdateCopyWithImpl;
  @override
  @useResult
  $Res call({String version});
}

/// @nodoc
class _$BridgeShareVersions_UpdateCopyWithImpl<$Res>
    implements $BridgeShareVersions_UpdateCopyWith<$Res> {
  _$BridgeShareVersions_UpdateCopyWithImpl(this._self, this._then);

  final BridgeShareVersions_Update _self;
  final $Res Function(BridgeShareVersions_Update) _then;

  /// Create a copy of BridgeShareVersions
  /// with the given fields replaced by the non-null parameter values.
  @override
  @pragma('vm:prefer-inline')
  $Res call({
    Object? version = null,
  }) {
    return _then(BridgeShareVersions_Update(
      version: null == version
          ? _self.version
          : version // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeShareVersions_Movable extends BridgeShareVersions {
  const BridgeShareVersions_Movable({required this.version}) : super._();

  @override
  final String version;

  /// Create a copy of BridgeShareVersions
  /// with the given fields replaced by the non-null parameter values.
  @override
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareVersions_MovableCopyWith<BridgeShareVersions_Movable>
      get copyWith => _$BridgeShareVersions_MovableCopyWithImpl<
          BridgeShareVersions_Movable>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareVersions_Movable &&
            (identical(other.version, version) || other.version == version));
  }

  @override
  int get hashCode => Object.hash(runtimeType, version);

  @override
  String toString() {
    return 'BridgeShareVersions.movable(version: $version)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareVersions_MovableCopyWith<$Res>
    implements $BridgeShareVersionsCopyWith<$Res> {
  factory $BridgeShareVersions_MovableCopyWith(
          BridgeShareVersions_Movable value,
          $Res Function(BridgeShareVersions_Movable) _then) =
      _$BridgeShareVersions_MovableCopyWithImpl;
  @override
  @useResult
  $Res call({String version});
}

/// @nodoc
class _$BridgeShareVersions_MovableCopyWithImpl<$Res>
    implements $BridgeShareVersions_MovableCopyWith<$Res> {
  _$BridgeShareVersions_MovableCopyWithImpl(this._self, this._then);

  final BridgeShareVersions_Movable _self;
  final $Res Function(BridgeShareVersions_Movable) _then;

  /// Create a copy of BridgeShareVersions
  /// with the given fields replaced by the non-null parameter values.
  @override
  @pragma('vm:prefer-inline')
  $Res call({
    Object? version = null,
  }) {
    return _then(BridgeShareVersions_Movable(
      version: null == version
          ? _self.version
          : version // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class BridgeShareVersions_HostFirst extends BridgeShareVersions {
  const BridgeShareVersions_HostFirst({required this.version}) : super._();

  @override
  final String version;

  /// Create a copy of BridgeShareVersions
  /// with the given fields replaced by the non-null parameter values.
  @override
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeShareVersions_HostFirstCopyWith<BridgeShareVersions_HostFirst>
      get copyWith => _$BridgeShareVersions_HostFirstCopyWithImpl<
          BridgeShareVersions_HostFirst>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeShareVersions_HostFirst &&
            (identical(other.version, version) || other.version == version));
  }

  @override
  int get hashCode => Object.hash(runtimeType, version);

  @override
  String toString() {
    return 'BridgeShareVersions.hostFirst(version: $version)';
  }
}

/// @nodoc
abstract mixin class $BridgeShareVersions_HostFirstCopyWith<$Res>
    implements $BridgeShareVersionsCopyWith<$Res> {
  factory $BridgeShareVersions_HostFirstCopyWith(
          BridgeShareVersions_HostFirst value,
          $Res Function(BridgeShareVersions_HostFirst) _then) =
      _$BridgeShareVersions_HostFirstCopyWithImpl;
  @override
  @useResult
  $Res call({String version});
}

/// @nodoc
class _$BridgeShareVersions_HostFirstCopyWithImpl<$Res>
    implements $BridgeShareVersions_HostFirstCopyWith<$Res> {
  _$BridgeShareVersions_HostFirstCopyWithImpl(this._self, this._then);

  final BridgeShareVersions_HostFirst _self;
  final $Res Function(BridgeShareVersions_HostFirst) _then;

  /// Create a copy of BridgeShareVersions
  /// with the given fields replaced by the non-null parameter values.
  @override
  @pragma('vm:prefer-inline')
  $Res call({
    Object? version = null,
  }) {
    return _then(BridgeShareVersions_HostFirst(
      version: null == version
          ? _self.version
          : version // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

// dart format on
