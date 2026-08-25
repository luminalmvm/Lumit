// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'state.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;

/// @nodoc
mixin _$WorkerResponse {
  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is WorkerResponse);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'WorkerResponse()';
  }
}

/// @nodoc
class $WorkerResponseCopyWith<$Res> {
  $WorkerResponseCopyWith(WorkerResponse _, $Res Function(WorkerResponse) __);
}

/// Adds pattern-matching-related methods to [WorkerResponse].
extension WorkerResponsePatterns on WorkerResponse {
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
    TResult Function(WorkerResponse_RenderedDMABuf value)? renderedDmaBuf,
    TResult Function(WorkerResponse_RenderedSharedTexture value)?
        renderedSharedTexture,
    TResult Function(WorkerResponse_Scope value)? scope,
    TResult Function(WorkerResponse_Sampled value)? sampled,
    TResult Function(WorkerResponse_PlaybackEnded value)? playbackEnded,
    TResult Function(WorkerResponse_CacheFilled value)? cacheFilled,
    TResult Function(WorkerResponse_RenderProgress value)? renderProgress,
    TResult Function(WorkerResponse_FrameProfile value)? frameProfile,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case WorkerResponse_RenderedDMABuf() when renderedDmaBuf != null:
        return renderedDmaBuf(_that);
      case WorkerResponse_RenderedSharedTexture()
          when renderedSharedTexture != null:
        return renderedSharedTexture(_that);
      case WorkerResponse_Scope() when scope != null:
        return scope(_that);
      case WorkerResponse_Sampled() when sampled != null:
        return sampled(_that);
      case WorkerResponse_PlaybackEnded() when playbackEnded != null:
        return playbackEnded(_that);
      case WorkerResponse_CacheFilled() when cacheFilled != null:
        return cacheFilled(_that);
      case WorkerResponse_RenderProgress() when renderProgress != null:
        return renderProgress(_that);
      case WorkerResponse_FrameProfile() when frameProfile != null:
        return frameProfile(_that);
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
    required TResult Function(WorkerResponse_RenderedDMABuf value)
        renderedDmaBuf,
    required TResult Function(WorkerResponse_RenderedSharedTexture value)
        renderedSharedTexture,
    required TResult Function(WorkerResponse_Scope value) scope,
    required TResult Function(WorkerResponse_Sampled value) sampled,
    required TResult Function(WorkerResponse_PlaybackEnded value) playbackEnded,
    required TResult Function(WorkerResponse_CacheFilled value) cacheFilled,
    required TResult Function(WorkerResponse_RenderProgress value)
        renderProgress,
    required TResult Function(WorkerResponse_FrameProfile value) frameProfile,
  }) {
    final _that = this;
    switch (_that) {
      case WorkerResponse_RenderedDMABuf():
        return renderedDmaBuf(_that);
      case WorkerResponse_RenderedSharedTexture():
        return renderedSharedTexture(_that);
      case WorkerResponse_Scope():
        return scope(_that);
      case WorkerResponse_Sampled():
        return sampled(_that);
      case WorkerResponse_PlaybackEnded():
        return playbackEnded(_that);
      case WorkerResponse_CacheFilled():
        return cacheFilled(_that);
      case WorkerResponse_RenderProgress():
        return renderProgress(_that);
      case WorkerResponse_FrameProfile():
        return frameProfile(_that);
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
    TResult? Function(WorkerResponse_RenderedDMABuf value)? renderedDmaBuf,
    TResult? Function(WorkerResponse_RenderedSharedTexture value)?
        renderedSharedTexture,
    TResult? Function(WorkerResponse_Scope value)? scope,
    TResult? Function(WorkerResponse_Sampled value)? sampled,
    TResult? Function(WorkerResponse_PlaybackEnded value)? playbackEnded,
    TResult? Function(WorkerResponse_CacheFilled value)? cacheFilled,
    TResult? Function(WorkerResponse_RenderProgress value)? renderProgress,
    TResult? Function(WorkerResponse_FrameProfile value)? frameProfile,
  }) {
    final _that = this;
    switch (_that) {
      case WorkerResponse_RenderedDMABuf() when renderedDmaBuf != null:
        return renderedDmaBuf(_that);
      case WorkerResponse_RenderedSharedTexture()
          when renderedSharedTexture != null:
        return renderedSharedTexture(_that);
      case WorkerResponse_Scope() when scope != null:
        return scope(_that);
      case WorkerResponse_Sampled() when sampled != null:
        return sampled(_that);
      case WorkerResponse_PlaybackEnded() when playbackEnded != null:
        return playbackEnded(_that);
      case WorkerResponse_CacheFilled() when cacheFilled != null:
        return cacheFilled(_that);
      case WorkerResponse_RenderProgress() when renderProgress != null:
        return renderProgress(_that);
      case WorkerResponse_FrameProfile() when frameProfile != null:
        return frameProfile(_that);
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
    TResult Function(BridgeSharedFrameInfoLinux field0)? renderedDmaBuf,
    TResult Function(BridgeSharedFrameInfo field0)? renderedSharedTexture,
    TResult Function(BridgeScopeTrace field0)? scope,
    TResult Function(BridgeSampledPixels field0)? sampled,
    TResult Function()? playbackEnded,
    TResult Function()? cacheFilled,
    TResult Function(BridgeRenderProgress field0)? renderProgress,
    TResult Function(BridgeFrameProfile field0)? frameProfile,
    required TResult orElse(),
  }) {
    final _that = this;
    switch (_that) {
      case WorkerResponse_RenderedDMABuf() when renderedDmaBuf != null:
        return renderedDmaBuf(_that.field0);
      case WorkerResponse_RenderedSharedTexture()
          when renderedSharedTexture != null:
        return renderedSharedTexture(_that.field0);
      case WorkerResponse_Scope() when scope != null:
        return scope(_that.field0);
      case WorkerResponse_Sampled() when sampled != null:
        return sampled(_that.field0);
      case WorkerResponse_PlaybackEnded() when playbackEnded != null:
        return playbackEnded();
      case WorkerResponse_CacheFilled() when cacheFilled != null:
        return cacheFilled();
      case WorkerResponse_RenderProgress() when renderProgress != null:
        return renderProgress(_that.field0);
      case WorkerResponse_FrameProfile() when frameProfile != null:
        return frameProfile(_that.field0);
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
    required TResult Function(BridgeSharedFrameInfoLinux field0) renderedDmaBuf,
    required TResult Function(BridgeSharedFrameInfo field0)
        renderedSharedTexture,
    required TResult Function(BridgeScopeTrace field0) scope,
    required TResult Function(BridgeSampledPixels field0) sampled,
    required TResult Function() playbackEnded,
    required TResult Function() cacheFilled,
    required TResult Function(BridgeRenderProgress field0) renderProgress,
    required TResult Function(BridgeFrameProfile field0) frameProfile,
  }) {
    final _that = this;
    switch (_that) {
      case WorkerResponse_RenderedDMABuf():
        return renderedDmaBuf(_that.field0);
      case WorkerResponse_RenderedSharedTexture():
        return renderedSharedTexture(_that.field0);
      case WorkerResponse_Scope():
        return scope(_that.field0);
      case WorkerResponse_Sampled():
        return sampled(_that.field0);
      case WorkerResponse_PlaybackEnded():
        return playbackEnded();
      case WorkerResponse_CacheFilled():
        return cacheFilled();
      case WorkerResponse_RenderProgress():
        return renderProgress(_that.field0);
      case WorkerResponse_FrameProfile():
        return frameProfile(_that.field0);
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
    TResult? Function(BridgeSharedFrameInfoLinux field0)? renderedDmaBuf,
    TResult? Function(BridgeSharedFrameInfo field0)? renderedSharedTexture,
    TResult? Function(BridgeScopeTrace field0)? scope,
    TResult? Function(BridgeSampledPixels field0)? sampled,
    TResult? Function()? playbackEnded,
    TResult? Function()? cacheFilled,
    TResult? Function(BridgeRenderProgress field0)? renderProgress,
    TResult? Function(BridgeFrameProfile field0)? frameProfile,
  }) {
    final _that = this;
    switch (_that) {
      case WorkerResponse_RenderedDMABuf() when renderedDmaBuf != null:
        return renderedDmaBuf(_that.field0);
      case WorkerResponse_RenderedSharedTexture()
          when renderedSharedTexture != null:
        return renderedSharedTexture(_that.field0);
      case WorkerResponse_Scope() when scope != null:
        return scope(_that.field0);
      case WorkerResponse_Sampled() when sampled != null:
        return sampled(_that.field0);
      case WorkerResponse_PlaybackEnded() when playbackEnded != null:
        return playbackEnded();
      case WorkerResponse_CacheFilled() when cacheFilled != null:
        return cacheFilled();
      case WorkerResponse_RenderProgress() when renderProgress != null:
        return renderProgress(_that.field0);
      case WorkerResponse_FrameProfile() when frameProfile != null:
        return frameProfile(_that.field0);
      case _:
        return null;
    }
  }
}

/// @nodoc

class WorkerResponse_RenderedDMABuf extends WorkerResponse {
  const WorkerResponse_RenderedDMABuf(this.field0) : super._();

  final BridgeSharedFrameInfoLinux field0;

  /// Create a copy of WorkerResponse
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $WorkerResponse_RenderedDMABufCopyWith<WorkerResponse_RenderedDMABuf>
      get copyWith => _$WorkerResponse_RenderedDMABufCopyWithImpl<
          WorkerResponse_RenderedDMABuf>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is WorkerResponse_RenderedDMABuf &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'WorkerResponse.renderedDmaBuf(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $WorkerResponse_RenderedDMABufCopyWith<$Res>
    implements $WorkerResponseCopyWith<$Res> {
  factory $WorkerResponse_RenderedDMABufCopyWith(
          WorkerResponse_RenderedDMABuf value,
          $Res Function(WorkerResponse_RenderedDMABuf) _then) =
      _$WorkerResponse_RenderedDMABufCopyWithImpl;
  @useResult
  $Res call({BridgeSharedFrameInfoLinux field0});
}

/// @nodoc
class _$WorkerResponse_RenderedDMABufCopyWithImpl<$Res>
    implements $WorkerResponse_RenderedDMABufCopyWith<$Res> {
  _$WorkerResponse_RenderedDMABufCopyWithImpl(this._self, this._then);

  final WorkerResponse_RenderedDMABuf _self;
  final $Res Function(WorkerResponse_RenderedDMABuf) _then;

  /// Create a copy of WorkerResponse
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(WorkerResponse_RenderedDMABuf(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeSharedFrameInfoLinux,
    ));
  }
}

/// @nodoc

class WorkerResponse_RenderedSharedTexture extends WorkerResponse {
  const WorkerResponse_RenderedSharedTexture(this.field0) : super._();

  final BridgeSharedFrameInfo field0;

  /// Create a copy of WorkerResponse
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $WorkerResponse_RenderedSharedTextureCopyWith<
          WorkerResponse_RenderedSharedTexture>
      get copyWith => _$WorkerResponse_RenderedSharedTextureCopyWithImpl<
          WorkerResponse_RenderedSharedTexture>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is WorkerResponse_RenderedSharedTexture &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'WorkerResponse.renderedSharedTexture(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $WorkerResponse_RenderedSharedTextureCopyWith<$Res>
    implements $WorkerResponseCopyWith<$Res> {
  factory $WorkerResponse_RenderedSharedTextureCopyWith(
          WorkerResponse_RenderedSharedTexture value,
          $Res Function(WorkerResponse_RenderedSharedTexture) _then) =
      _$WorkerResponse_RenderedSharedTextureCopyWithImpl;
  @useResult
  $Res call({BridgeSharedFrameInfo field0});
}

/// @nodoc
class _$WorkerResponse_RenderedSharedTextureCopyWithImpl<$Res>
    implements $WorkerResponse_RenderedSharedTextureCopyWith<$Res> {
  _$WorkerResponse_RenderedSharedTextureCopyWithImpl(this._self, this._then);

  final WorkerResponse_RenderedSharedTexture _self;
  final $Res Function(WorkerResponse_RenderedSharedTexture) _then;

  /// Create a copy of WorkerResponse
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(WorkerResponse_RenderedSharedTexture(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeSharedFrameInfo,
    ));
  }
}

/// @nodoc

class WorkerResponse_Scope extends WorkerResponse {
  const WorkerResponse_Scope(this.field0) : super._();

  final BridgeScopeTrace field0;

  /// Create a copy of WorkerResponse
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $WorkerResponse_ScopeCopyWith<WorkerResponse_Scope> get copyWith =>
      _$WorkerResponse_ScopeCopyWithImpl<WorkerResponse_Scope>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is WorkerResponse_Scope &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'WorkerResponse.scope(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $WorkerResponse_ScopeCopyWith<$Res>
    implements $WorkerResponseCopyWith<$Res> {
  factory $WorkerResponse_ScopeCopyWith(WorkerResponse_Scope value,
          $Res Function(WorkerResponse_Scope) _then) =
      _$WorkerResponse_ScopeCopyWithImpl;
  @useResult
  $Res call({BridgeScopeTrace field0});
}

/// @nodoc
class _$WorkerResponse_ScopeCopyWithImpl<$Res>
    implements $WorkerResponse_ScopeCopyWith<$Res> {
  _$WorkerResponse_ScopeCopyWithImpl(this._self, this._then);

  final WorkerResponse_Scope _self;
  final $Res Function(WorkerResponse_Scope) _then;

  /// Create a copy of WorkerResponse
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(WorkerResponse_Scope(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeScopeTrace,
    ));
  }
}

/// @nodoc

class WorkerResponse_Sampled extends WorkerResponse {
  const WorkerResponse_Sampled(this.field0) : super._();

  final BridgeSampledPixels field0;

  /// Create a copy of WorkerResponse
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $WorkerResponse_SampledCopyWith<WorkerResponse_Sampled> get copyWith =>
      _$WorkerResponse_SampledCopyWithImpl<WorkerResponse_Sampled>(
          this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is WorkerResponse_Sampled &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'WorkerResponse.sampled(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $WorkerResponse_SampledCopyWith<$Res>
    implements $WorkerResponseCopyWith<$Res> {
  factory $WorkerResponse_SampledCopyWith(WorkerResponse_Sampled value,
          $Res Function(WorkerResponse_Sampled) _then) =
      _$WorkerResponse_SampledCopyWithImpl;
  @useResult
  $Res call({BridgeSampledPixels field0});
}

/// @nodoc
class _$WorkerResponse_SampledCopyWithImpl<$Res>
    implements $WorkerResponse_SampledCopyWith<$Res> {
  _$WorkerResponse_SampledCopyWithImpl(this._self, this._then);

  final WorkerResponse_Sampled _self;
  final $Res Function(WorkerResponse_Sampled) _then;

  /// Create a copy of WorkerResponse
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(WorkerResponse_Sampled(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeSampledPixels,
    ));
  }
}

/// @nodoc

class WorkerResponse_PlaybackEnded extends WorkerResponse {
  const WorkerResponse_PlaybackEnded() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is WorkerResponse_PlaybackEnded);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'WorkerResponse.playbackEnded()';
  }
}

/// @nodoc

class WorkerResponse_CacheFilled extends WorkerResponse {
  const WorkerResponse_CacheFilled() : super._();

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is WorkerResponse_CacheFilled);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  String toString() {
    return 'WorkerResponse.cacheFilled()';
  }
}

/// @nodoc

class WorkerResponse_RenderProgress extends WorkerResponse {
  const WorkerResponse_RenderProgress(this.field0) : super._();

  final BridgeRenderProgress field0;

  /// Create a copy of WorkerResponse
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $WorkerResponse_RenderProgressCopyWith<WorkerResponse_RenderProgress>
      get copyWith => _$WorkerResponse_RenderProgressCopyWithImpl<
          WorkerResponse_RenderProgress>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is WorkerResponse_RenderProgress &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'WorkerResponse.renderProgress(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $WorkerResponse_RenderProgressCopyWith<$Res>
    implements $WorkerResponseCopyWith<$Res> {
  factory $WorkerResponse_RenderProgressCopyWith(
          WorkerResponse_RenderProgress value,
          $Res Function(WorkerResponse_RenderProgress) _then) =
      _$WorkerResponse_RenderProgressCopyWithImpl;
  @useResult
  $Res call({BridgeRenderProgress field0});
}

/// @nodoc
class _$WorkerResponse_RenderProgressCopyWithImpl<$Res>
    implements $WorkerResponse_RenderProgressCopyWith<$Res> {
  _$WorkerResponse_RenderProgressCopyWithImpl(this._self, this._then);

  final WorkerResponse_RenderProgress _self;
  final $Res Function(WorkerResponse_RenderProgress) _then;

  /// Create a copy of WorkerResponse
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(WorkerResponse_RenderProgress(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeRenderProgress,
    ));
  }
}

/// @nodoc

class WorkerResponse_FrameProfile extends WorkerResponse {
  const WorkerResponse_FrameProfile(this.field0) : super._();

  final BridgeFrameProfile field0;

  /// Create a copy of WorkerResponse
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @pragma('vm:prefer-inline')
  $WorkerResponse_FrameProfileCopyWith<WorkerResponse_FrameProfile>
      get copyWith => _$WorkerResponse_FrameProfileCopyWithImpl<
          WorkerResponse_FrameProfile>(this, _$identity);

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is WorkerResponse_FrameProfile &&
            (identical(other.field0, field0) || other.field0 == field0));
  }

  @override
  int get hashCode => Object.hash(runtimeType, field0);

  @override
  String toString() {
    return 'WorkerResponse.frameProfile(field0: $field0)';
  }
}

/// @nodoc
abstract mixin class $WorkerResponse_FrameProfileCopyWith<$Res>
    implements $WorkerResponseCopyWith<$Res> {
  factory $WorkerResponse_FrameProfileCopyWith(
          WorkerResponse_FrameProfile value,
          $Res Function(WorkerResponse_FrameProfile) _then) =
      _$WorkerResponse_FrameProfileCopyWithImpl;
  @useResult
  $Res call({BridgeFrameProfile field0});
}

/// @nodoc
class _$WorkerResponse_FrameProfileCopyWithImpl<$Res>
    implements $WorkerResponse_FrameProfileCopyWith<$Res> {
  _$WorkerResponse_FrameProfileCopyWithImpl(this._self, this._then);

  final WorkerResponse_FrameProfile _self;
  final $Res Function(WorkerResponse_FrameProfile) _then;

  /// Create a copy of WorkerResponse
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  $Res call({
    Object? field0 = null,
  }) {
    return _then(WorkerResponse_FrameProfile(
      null == field0
          ? _self.field0
          : field0 // ignore: cast_nullable_to_non_nullable
              as BridgeFrameProfile,
    ));
  }
}

// dart format on
