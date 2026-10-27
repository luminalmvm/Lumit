/*
 * Sort: an OFX plugin that sorts runs of pixels by brightness, along rows or
 * columns. A run is a stretch of pixels brighter than Threshold. Darkest
 * first, or brightest first with Reverse.
 *
 * One C file against the OpenFX headers in openfx/ (BSD licence, from
 * github.com/AcademySoftwareFoundation/openfx). Build it into the bundle the
 * manifest names, from this folder:
 *
 *   Windows, in an x64 Native Tools prompt:
 *     mkdir Sort.ofx.bundle\Contents\Win64
 *     cl /nologo /O2 /LD sort.c /Fe:Sort.ofx.bundle\Contents\Win64\Sort.ofx /link /NOIMPLIB /NOEXP
 *
 *   macOS, for both kinds of Mac:
 *     mkdir -p Sort.ofx.bundle/Contents/MacOS
 *     clang -O2 -bundle -arch arm64 -arch x86_64 sort.c -o Sort.ofx.bundle/Contents/MacOS/Sort.ofx
 *
 *   Linux:
 *     mkdir -p Sort.ofx.bundle/Contents/Linux-x86-64
 *     gcc -O2 -shared -fPIC sort.c -o Sort.ofx.bundle/Contents/Linux-x86-64/Sort.ofx
 *
 * Then install this folder, or make a package of it, from Settings > Addons >
 * Extensions. A package can hold all three binaries at once.
 */

#include <stdlib.h>
#include <string.h>

#include "openfx/ofxImageEffect.h"

static OfxHost *host;
static const OfxPropertySuiteV1 *props;
static const OfxImageEffectSuiteV1 *fx;
static const OfxParameterSuiteV1 *params;

/* A pixel is four floats, red, green, blue and alpha. */
enum { PIXEL = 4 * sizeof(float) };

static float brightness(const float *p) {
  return 0.2126f * p[0] + 0.7152f * p[1] + 0.0722f * p[2];
}

/* Darker first. Ties go by the channels, so every C library sorts alike. */
static int darker(const void *x, const void *y) {
  const float *a = x, *b = y;
  float la = brightness(a), lb = brightness(b);
  if (la != lb) return la < lb ? -1 : 1;
  for (int i = 0; i < 4; i++)
    if (a[i] != b[i]) return a[i] < b[i] ? -1 : 1;
  return 0;
}

/* Sort the runs along one line of n pixels, step bytes apart, using run as
 * room for the longest. */
static void sort_line(char *line, ptrdiff_t step, int n, float threshold,
                      int reverse, float *run) {
  int start = 0;
  while (start < n) {
    int end = start;
    while (end < n && brightness((float *)(line + end * step)) > threshold) {
      memcpy(run + (end - start) * 4, line + end * step, PIXEL);
      end++;
    }
    int len = end - start;
    qsort(run, len, PIXEL, darker);
    for (int i = 0; i < len; i++)
      memcpy(line + (start + i) * step, run + (reverse ? len - 1 - i : i) * 4, PIXEL);
    start = end + 1;
  }
}

/* An image's pixels, where they sit and how far apart its rows are. Rows can
 * run backwards in memory, so row_bytes may be negative. */
typedef struct {
  char *data;
  OfxRectI bounds;
  int row_bytes;
} View;

static int view(OfxPropertySetHandle image, View *v) {
  void *data = NULL;
  props->propGetPointer(image, kOfxImagePropData, 0, &data);
  props->propGetIntN(image, kOfxImagePropBounds, 4, &v->bounds.x1);
  props->propGetInt(image, kOfxImagePropRowBytes, 0, &v->row_bytes);
  v->data = data;
  return data != NULL;
}

static int holds(const View *v, int x, int y) {
  return x >= v->bounds.x1 && x < v->bounds.x2 && y >= v->bounds.y1 && y < v->bounds.y2;
}

static char *at(const View *v, int x, int y) {
  return v->data + (ptrdiff_t)(y - v->bounds.y1) * v->row_bytes + (ptrdiff_t)(x - v->bounds.x1) * PIXEL;
}

static OfxStatus sort_image(OfxImageEffectHandle effect, OfxPropertySetHandle source,
                            OfxPropertySetHandle output, OfxRectI w, int columns,
                            int reverse, float threshold) {
  View s, d;
  if (!view(source, &s) || !view(output, &d)) return kOfxStatFailed;
  /* The window, held to the output. */
  if (w.x1 < d.bounds.x1) w.x1 = d.bounds.x1;
  if (w.y1 < d.bounds.y1) w.y1 = d.bounds.y1;
  if (w.x2 > d.bounds.x2) w.x2 = d.bounds.x2;
  if (w.y2 > d.bounds.y2) w.y2 = d.bounds.y2;
  int width = w.x2 - w.x1, height = w.y2 - w.y1;
  if (width <= 0 || height <= 0) return kOfxStatOK;

  /* The source as it is, transparent where it ends, then sorted in place. */
  for (int y = w.y1; y < w.y2; y++)
    for (int x = w.x1; x < w.x2; x++) {
      if (holds(&s, x, y)) memcpy(at(&d, x, y), at(&s, x, y), PIXEL);
      else memset(at(&d, x, y), 0, PIXEL);
    }

  float *run = malloc((size_t)(width > height ? width : height) * PIXEL);
  if (!run) return kOfxStatErrMemory;
  if (columns) {
    for (int x = w.x1; x < w.x2 && !fx->abort(effect); x++)
      sort_line(at(&d, x, w.y1), d.row_bytes, height, threshold, reverse, run);
  } else {
    for (int y = w.y1; y < w.y2 && !fx->abort(effect); y++)
      sort_line(at(&d, w.x1, y), PIXEL, width, threshold, reverse, run);
  }
  free(run);
  return kOfxStatOK;
}

static OfxStatus render(OfxImageEffectHandle effect, OfxPropertySetHandle args) {
  OfxTime time = 0;
  OfxRectI window;
  props->propGetDouble(args, kOfxPropTime, 0, &time);
  props->propGetIntN(args, kOfxImageEffectPropRenderWindow, 4, &window.x1);

  OfxParamSetHandle set;
  OfxParamHandle param;
  int columns = 0, reverse = 0;
  double threshold = 0.25;
  fx->getParamSet(effect, &set);
  if (params->paramGetHandle(set, "direction", &param, NULL) == kOfxStatOK)
    params->paramGetValueAtTime(param, time, &columns);
  if (params->paramGetHandle(set, "threshold", &param, NULL) == kOfxStatOK)
    params->paramGetValueAtTime(param, time, &threshold);
  if (params->paramGetHandle(set, "reverse", &param, NULL) == kOfxStatOK)
    params->paramGetValueAtTime(param, time, &reverse);

  OfxImageClipHandle clip;
  OfxPropertySetHandle source = NULL, output = NULL;
  if (fx->clipGetHandle(effect, kOfxImageEffectSimpleSourceClipName, &clip, NULL) == kOfxStatOK)
    fx->clipGetImage(clip, time, NULL, &source);
  if (fx->clipGetHandle(effect, kOfxImageEffectOutputClipName, &clip, NULL) == kOfxStatOK)
    fx->clipGetImage(clip, time, NULL, &output);
  OfxStatus status = kOfxStatFailed;
  if (source && output)
    status = sort_image(effect, source, output, window, columns, reverse, (float)threshold);
  if (source) fx->clipReleaseImage(source);
  if (output) fx->clipReleaseImage(output);
  return status;
}

static OfxStatus describe(OfxImageEffectHandle effect) {
  OfxPropertySetHandle p;
  fx->getPropertySet(effect, &p);
  props->propSetString(p, kOfxPropLabel, 0, "Sort");
  props->propSetString(p, kOfxImageEffectPluginPropGrouping, 0, "Stylise");
  props->propSetString(p, kOfxImageEffectPropSupportedContexts, 0, kOfxImageEffectContextFilter);
  props->propSetString(p, kOfxImageEffectPropSupportedPixelDepths, 0, kOfxBitDepthFloat);
  props->propSetString(p, kOfxImageEffectPluginRenderThreadSafety, 0, kOfxImageEffectRenderFullySafe);
  /* A run can cross any tile, so it takes whole frames. */
  props->propSetInt(p, kOfxImageEffectPropSupportsTiles, 0, 0);
  return kOfxStatOK;
}

static OfxStatus describe_in_context(OfxImageEffectHandle effect) {
  OfxPropertySetHandle p;
  fx->clipDefine(effect, kOfxImageEffectSimpleSourceClipName, &p);
  props->propSetString(p, kOfxImageEffectPropSupportedComponents, 0, kOfxImageComponentRGBA);
  fx->clipDefine(effect, kOfxImageEffectOutputClipName, &p);
  props->propSetString(p, kOfxImageEffectPropSupportedComponents, 0, kOfxImageComponentRGBA);

  OfxParamSetHandle set;
  fx->getParamSet(effect, &set);
  params->paramDefine(set, kOfxParamTypeChoice, "direction", &p);
  props->propSetString(p, kOfxPropLabel, 0, "Direction");
  props->propSetString(p, kOfxParamPropChoiceOption, 0, "Rows");
  props->propSetString(p, kOfxParamPropChoiceOption, 1, "Columns");
  params->paramDefine(set, kOfxParamTypeDouble, "threshold", &p);
  props->propSetString(p, kOfxPropLabel, 0, "Threshold");
  props->propSetDouble(p, kOfxParamPropDefault, 0, 0.25);
  props->propSetDouble(p, kOfxParamPropMin, 0, 0);
  props->propSetDouble(p, kOfxParamPropMax, 0, 1);
  props->propSetDouble(p, kOfxParamPropDisplayMin, 0, 0);
  props->propSetDouble(p, kOfxParamPropDisplayMax, 0, 1);
  params->paramDefine(set, kOfxParamTypeBoolean, "reverse", &p);
  props->propSetString(p, kOfxPropLabel, 0, "Reverse");
  return kOfxStatOK;
}

static OfxStatus load(void) {
  props = host->fetchSuite(host->host, kOfxPropertySuite, 1);
  fx = host->fetchSuite(host->host, kOfxImageEffectSuite, 1);
  params = host->fetchSuite(host->host, kOfxParameterSuite, 1);
  return props && fx && params ? kOfxStatOK : kOfxStatErrMissingHostFeature;
}

static OfxStatus main_entry(const char *action, const void *handle,
                            OfxPropertySetHandle in_args, OfxPropertySetHandle out_args) {
  OfxImageEffectHandle effect = (OfxImageEffectHandle)handle;
  (void)out_args;
  if (!strcmp(action, kOfxActionLoad)) return load();
  if (!strcmp(action, kOfxActionDescribe)) return describe(effect);
  if (!strcmp(action, kOfxImageEffectActionDescribeInContext)) return describe_in_context(effect);
  if (!strcmp(action, kOfxImageEffectActionRender)) return render(effect, in_args);
  return kOfxStatReplyDefault;
}

static void set_host(OfxHost *given) { host = given; }

static OfxPlugin plugin = {
    kOfxImageEffectPluginApi, 1, "com.lumitlab.sort", 1, 0, set_host, main_entry,
};

OfxExport int OfxGetNumberOfPlugins(void) { return 1; }

OfxExport OfxPlugin *OfxGetPlugin(int nth) { return nth == 0 ? &plugin : NULL; }
