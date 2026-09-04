/*
 * The four parameter-suite entry points that are C-variadic in the OFX header.
 *
 * In plain terms: `paramGetValue(handle, ...)` takes a different number of
 * trailing arguments, one per dimension of the parameter, and `paramSetValue`
 * takes them as values whose type depends on the parameter. Rust cannot define
 * a function like that on stable (rust-lang#44930), and declaring one with a
 * fixed number of arguments instead only works where the calling convention
 * happens to put variadic and fixed arguments in the same place. It does on
 * x86-64 Windows; it does not on Apple silicon, where variadic arguments go on
 * the stack while fixed ones stay in registers, so a plugin's out-pointer would
 * be read from the wrong place and written through.
 *
 * So these four are C, and C only does what C can do and Rust cannot: it asks
 * Rust what shape the parameter is, pulls exactly that many arguments off the
 * variadic list with `va_arg`, and hands them to a fixed-arity Rust function.
 * Every decision (which handle is live, what a value means, where it goes)
 * stays in Rust. A parameter Rust does not recognise has a count of nought, so
 * nothing is pulled and the Rust half answers with the right refusal.
 *
 * The Rust functions are reached through pointers Rust installs with
 * `lumit_ofx_variadic_bind`, not by name. By name would be `#[no_mangle]`, and
 * the host's own test binaries link the crate twice, once under test and once
 * beneath the test plugin, which borrows its C declarations, so a named export
 * is defined twice and the link fails. Rust binds the moment its host state is
 * first built, which every suite call reaches before a plugin can hold the
 * parameter handle it would need to call one of these.
 *
 * Built by `build.rs`; the Rust side is `suites/parameter.rs`.
 */

#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

typedef void *OfxParamHandle;
typedef double OfxTime;
typedef int OfxStatus;

/* `kOfxStatErrFatal`: the answer if a call somehow arrives before Rust has
 * bound, which nothing in the host can do. */
#define LUMIT_OFX_UNBOUND 2

/* Mirrors `Reading` in parameter.rs; the numbers are the contract. */
enum { LUMIT_OFX_INT = 0, LUMIT_OFX_DOUBLE = 1, LUMIT_OFX_TEXT = 2 };

/* Rust: how a parameter's trailing arguments are typed and how many there are.
 * Never fails; a handle Rust cannot serve is a count of nought. */
typedef void (*lumit_ofx_shape_fn)(OfxParamHandle param, int *kind, int *count);
/* Rust: the fixed-arity halves. */
typedef OfxStatus (*lumit_ofx_get_fn)(OfxParamHandle param, void *v0, void *v1, void *v2,
                                      void *v3);
typedef OfxStatus (*lumit_ofx_set_fn)(OfxParamHandle param, uint64_t v0, uint64_t v1,
                                      uint64_t v2, uint64_t v3);

static lumit_ofx_shape_fn shape_fn = NULL;
static lumit_ofx_get_fn get_fn = NULL;
static lumit_ofx_set_fn set_fn = NULL;

void lumit_ofx_variadic_bind(lumit_ofx_shape_fn shape, lumit_ofx_get_fn get,
                             lumit_ofx_set_fn set) {
    shape_fn = shape;
    get_fn = get;
    set_fn = set;
}

static OfxStatus get_from(OfxParamHandle param, va_list ap) {
    int kind = 0, count = 0;
    void *slots[4] = {0, 0, 0, 0};
    if (shape_fn == NULL || get_fn == NULL) {
        return LUMIT_OFX_UNBOUND;
    }
    shape_fn(param, &kind, &count);
    for (int i = 0; i < count && i < 4; i++) {
        slots[i] = va_arg(ap, void *);
    }
    return get_fn(param, slots[0], slots[1], slots[2], slots[3]);
}

static OfxStatus set_from(OfxParamHandle param, va_list ap) {
    int kind = 0, count = 0;
    uint64_t slots[4] = {0, 0, 0, 0};
    if (shape_fn == NULL || set_fn == NULL) {
        return LUMIT_OFX_UNBOUND;
    }
    shape_fn(param, &kind, &count);
    for (int i = 0; i < count && i < 4; i++) {
        switch (kind) {
        case LUMIT_OFX_INT:
            /* An `int` is promoted to nothing wider in a variadic call. */
            slots[i] = (uint64_t)(uint32_t)va_arg(ap, int);
            break;
        case LUMIT_OFX_DOUBLE: {
            /* A `float` would have been promoted to `double` on the way in. */
            double d = va_arg(ap, double);
            memcpy(&slots[i], &d, sizeof d);
            break;
        }
        default:
            slots[i] = (uint64_t)(uintptr_t)va_arg(ap, const char *);
            break;
        }
    }
    return set_fn(param, slots[0], slots[1], slots[2], slots[3]);
}

OfxStatus lumit_ofx_param_get_value(OfxParamHandle param, ...) {
    va_list ap;
    va_start(ap, param);
    OfxStatus status = get_from(param, ap);
    va_end(ap);
    return status;
}

OfxStatus lumit_ofx_param_get_value_at_time(OfxParamHandle param, OfxTime time, ...) {
    /* The snapshot is one moment; see parameter.rs for why the time is not
     * looked at. */
    (void)time;
    va_list ap;
    va_start(ap, time);
    OfxStatus status = get_from(param, ap);
    va_end(ap);
    return status;
}

OfxStatus lumit_ofx_param_set_value(OfxParamHandle param, ...) {
    va_list ap;
    va_start(ap, param);
    OfxStatus status = set_from(param, ap);
    va_end(ap);
    return status;
}

OfxStatus lumit_ofx_param_set_value_at_time(OfxParamHandle param, OfxTime time, ...) {
    (void)time;
    va_list ap;
    va_start(ap, time);
    OfxStatus status = set_from(param, ap);
    va_end(ap);
    return status;
}
