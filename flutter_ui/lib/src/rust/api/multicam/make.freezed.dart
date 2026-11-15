// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'make.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$BridgeMulticamJob {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeMulticamJob);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeMulticamJob()';
  }
}

/// @nodoc
class $BridgeMulticamJobCopyWith<$Res> {
  $BridgeMulticamJobCopyWith(
      BridgeMulticamJob _, $Res Function(BridgeMulticamJob) __);
}

/// Adds pattern-matching-related methods to [BridgeMulticamJob].
extension BridgeMulticamJobPatterns on BridgeMulticamJob {
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
    TResult Function(BridgeMulticamJob_Idle value)? idle,
    TResult Function(BridgeMulticamJob_Working value)? working,
    TResult Function(BridgeMulticamJob_Done value)? done,
    TResult Function(BridgeMulticamJob_Failed value)? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeMulticamJob_Idle() when idle != null:
        return idle(_that);
      case BridgeMulticamJob_Working() when working != null:
        return working(_that);
      case BridgeMulticamJob_Done() when done != null:
        return done(_that);
      case BridgeMulticamJob_Failed() when failed != null:
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
    required TResult Function(BridgeMulticamJob_Idle value) idle,
    required TResult Function(BridgeMulticamJob_Working value) working,
    required TResult Function(BridgeMulticamJob_Done value) done,
    required TResult Function(BridgeMulticamJob_Failed value) failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeMulticamJob_Idle():
        return idle(_that);
      case BridgeMulticamJob_Working():
        return working(_that);
      case BridgeMulticamJob_Done():
        return done(_that);
      case BridgeMulticamJob_Failed():
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
    TResult? Function(BridgeMulticamJob_Idle value)? idle,
    TResult? Function(BridgeMulticamJob_Working value)? working,
    TResult? Function(BridgeMulticamJob_Done value)? done,
    TResult? Function(BridgeMulticamJob_Failed value)? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeMulticamJob_Idle() when idle != null:
        return idle(_that);
      case BridgeMulticamJob_Working() when working != null:
        return working(_that);
      case BridgeMulticamJob_Done() when done != null:
        return done(_that);
      case BridgeMulticamJob_Failed() when failed != null:
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
    TResult Function(double done)? working,
    TResult Function(UuidValue comp, int synced, List<String> uncertain,
            List<String> unsynced)?
        done,
    TResult Function()? failed,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case BridgeMulticamJob_Idle() when idle != null:
        return idle();
      case BridgeMulticamJob_Working() when working != null:
        return working(_that.done);
      case BridgeMulticamJob_Done() when done != null:
        return done(_that.comp, _that.synced, _that.uncertain, _that.unsynced);
      case BridgeMulticamJob_Failed() when failed != null:
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
    required TResult Function(double done) working,
    required TResult Function(UuidValue comp, int synced,
            List<String> uncertain, List<String> unsynced)
        done,
    required TResult Function() failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeMulticamJob_Idle():
        return idle();
      case BridgeMulticamJob_Working():
        return working(_that.done);
      case BridgeMulticamJob_Done():
        return done(_that.comp, _that.synced, _that.uncertain, _that.unsynced);
      case BridgeMulticamJob_Failed():
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
    TResult? Function(double done)? working,
    TResult? Function(UuidValue comp, int synced, List<String> uncertain,
            List<String> unsynced)?
        done,
    TResult? Function()? failed,
  }) {
    final _that = this;
    switch (_that) {
      case BridgeMulticamJob_Idle() when idle != null:
        return idle();
      case BridgeMulticamJob_Working() when working != null:
        return working(_that.done);
      case BridgeMulticamJob_Done() when done != null:
        return done(_that.comp, _that.synced, _that.uncertain, _that.unsynced);
      case BridgeMulticamJob_Failed() when failed != null:
        return failed();
      case _:
        return null;
    }
  }
}

/// @nodoc

class BridgeMulticamJob_Idle extends BridgeMulticamJob {
  const BridgeMulticamJob_Idle() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeMulticamJob_Idle);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeMulticamJob.idle()';
  }
}

/// @nodoc

class BridgeMulticamJob_Working extends BridgeMulticamJob {
  const BridgeMulticamJob_Working({required this.done}) : super._();

  final double done;

  /// Create a copy of BridgeMulticamJob
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeMulticamJob_WorkingCopyWith<BridgeMulticamJob_Working> get copyWith =>
      _$BridgeMulticamJob_WorkingCopyWithImpl<BridgeMulticamJob_Working>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeMulticamJob_Working &&
            (identical(other.done, done) || other.done == done));
  }

  @override
  int get hashCode => Object.hash(runtimeType, done);

  @override
  String toString() {
    return 'BridgeMulticamJob.working(done: $done)';
  }
}

/// @nodoc
abstract mixin class $BridgeMulticamJob_WorkingCopyWith<$Res>
    implements $BridgeMulticamJobCopyWith<$Res> {
  factory $BridgeMulticamJob_WorkingCopyWith(BridgeMulticamJob_Working value,
          $Res Function(BridgeMulticamJob_Working) _then) =
      _$BridgeMulticamJob_WorkingCopyWithImpl;
  @useResult
  $Res call({double done});
}

/// @nodoc
class _$BridgeMulticamJob_WorkingCopyWithImpl<$Res>
    implements $BridgeMulticamJob_WorkingCopyWith<$Res> {
  _$BridgeMulticamJob_WorkingCopyWithImpl(this._self, this._then);

  final BridgeMulticamJob_Working _self;
  final $Res Function(BridgeMulticamJob_Working) _then;

  /// Create a copy of BridgeMulticamJob
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? done = null,
  }) {
    return _then(BridgeMulticamJob_Working(
      done: null == done
          ? _self.done
          : done // ignore: cast_nullable_to_non_nullable
              as double,
    ));
  }
}

/// @nodoc

class BridgeMulticamJob_Done extends BridgeMulticamJob {
  const BridgeMulticamJob_Done(
      {required this.comp,
      required this.synced,
      required final List<String> uncertain,
      required final List<String> unsynced})
      : _uncertain = uncertain,
        _unsynced = unsynced,
        super._();

  final UuidValue comp;
  final int synced;
  final List<String> _uncertain;
  List<String> get uncertain {
    if (_uncertain is EqualUnmodifiableListView) return _uncertain;
    // ignore: implicit_dynamic_type
    return EqualUnmodifiableListView(_uncertain);
  }

  final List<String> _unsynced;
  List<String> get unsynced {
    if (_unsynced is EqualUnmodifiableListView) return _unsynced;
    // ignore: implicit_dynamic_type
    return EqualUnmodifiableListView(_unsynced);
  }

  /// Create a copy of BridgeMulticamJob
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $BridgeMulticamJob_DoneCopyWith<BridgeMulticamJob_Done> get copyWith =>
      _$BridgeMulticamJob_DoneCopyWithImpl<BridgeMulticamJob_Done>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is BridgeMulticamJob_Done &&
            (identical(other.comp, comp) || other.comp == comp) &&
            (identical(other.synced, synced) || other.synced == synced) &&
            const DeepCollectionEquality()
                .equals(other._uncertain, _uncertain) &&
            const DeepCollectionEquality().equals(other._unsynced, _unsynced));
  }

  @override
  int get hashCode => Object.hash(
      runtimeType,
      comp,
      synced,
      const DeepCollectionEquality().hash(_uncertain),
      const DeepCollectionEquality().hash(_unsynced));

  @override
  String toString() {
    return 'BridgeMulticamJob.done(comp: $comp, synced: $synced, uncertain: $uncertain, unsynced: $unsynced)';
  }
}

/// @nodoc
abstract mixin class $BridgeMulticamJob_DoneCopyWith<$Res>
    implements $BridgeMulticamJobCopyWith<$Res> {
  factory $BridgeMulticamJob_DoneCopyWith(BridgeMulticamJob_Done value,
          $Res Function(BridgeMulticamJob_Done) _then) =
      _$BridgeMulticamJob_DoneCopyWithImpl;
  @useResult
  $Res call(
      {UuidValue comp,
      int synced,
      List<String> uncertain,
      List<String> unsynced});
}

/// @nodoc
class _$BridgeMulticamJob_DoneCopyWithImpl<$Res>
    implements $BridgeMulticamJob_DoneCopyWith<$Res> {
  _$BridgeMulticamJob_DoneCopyWithImpl(this._self, this._then);

  final BridgeMulticamJob_Done _self;
  final $Res Function(BridgeMulticamJob_Done) _then;

  /// Create a copy of BridgeMulticamJob
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? comp = null,
    Object? synced = null,
    Object? uncertain = null,
    Object? unsynced = null,
  }) {
    return _then(BridgeMulticamJob_Done(
      comp: null == comp
          ? _self.comp
          : comp // ignore: cast_nullable_to_non_nullable
              as UuidValue,
      synced: null == synced
          ? _self.synced
          : synced // ignore: cast_nullable_to_non_nullable
              as int,
      uncertain: null == uncertain
          ? _self._uncertain
          : uncertain // ignore: cast_nullable_to_non_nullable
              as List<String>,
      unsynced: null == unsynced
          ? _self._unsynced
          : unsynced // ignore: cast_nullable_to_non_nullable
              as List<String>,
    ));
  }
}

/// @nodoc

class BridgeMulticamJob_Failed extends BridgeMulticamJob {
  const BridgeMulticamJob_Failed() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is BridgeMulticamJob_Failed);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'BridgeMulticamJob.failed()';
  }
}

// dart format on
