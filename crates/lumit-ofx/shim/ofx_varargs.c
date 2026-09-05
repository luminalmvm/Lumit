/* The four C-variadic entry points of `OfxParameterSuiteV1` (K-756).
 *
 * In plain terms. OFX declares `paramGetValue` and its three relatives with a
 * trailing `...`: a plugin passes one argument per dimension, so a double
 * passes one and an RGBA colour passes four. Rust cannot *define* such a
 * function on stable (rust-lang#44930, still true on 1.97), so this host used
 * to declare them with a fixed arity of four instead and read the arguments
 * from wherever a fixed call would have left them. That is only correct where
 * a variadic call and a fixed call agree about placement. Windows x64, System
 * V and standard AAPCS64 agree; Apple's arm64 ABI does not — it passes every
 * variadic argument on the stack — so the host read four registers of
 * leftovers, and on a read it then wrote the parameter's value through them.
 * No commercial plugin's parameters worked on an M-series Mac.
 *
 * This file is the whole fix, and it is deliberately the smallest thing that
 * could be. `va_arg` is the only construct that knows each platform's rule, so
 * this pulls the arguments and hands them to the Rust half as a fixed array.
 * What a value *means*, and every refusal, stays on the Rust side: there is no
 * status code and no parameter-type table in here, on purpose — a second copy
 * of either would drift from the first.
 */

#include <stdarg.h>
#include <stdint.h>
#include <string.h>

/* How the trailing arguments of a `paramSetValue` are typed. These are the
 * discriminants of `suites::parameter::Reading` and must stay in step with it;
 * `the_shim_and_its_rust_half_agree_about_readings` is the test that says so. */
#define LUMIT_OFX_READING_INT 0
#define LUMIT_OFX_READING_DOUBLE 1
#define LUMIT_OFX_READING_TEXT 2

/* The Rust half. Each is `#[no_mangle] pub extern "C"` over there. */
int lumit_ofx_param_arity(void *param, int *count, int *reading);
int lumit_ofx_param_get_value(void *param, void *v0, void *v1, void *v2, void *v3);
int lumit_ofx_param_get_value_at_time(void *param, double time, void *v0, void *v1,
                                      void *v2, void *v3);
int lumit_ofx_param_set_value(void *param, uint64_t w0, uint64_t w1, uint64_t w2,
                              uint64_t w3);
int lumit_ofx_param_set_value_at_time(void *param, double time, uint64_t w0, uint64_t w1,
                                      uint64_t w2, uint64_t w3);

/* ---------------------------------------------------------- reading values -- */

/* Pull `count` out-pointers into `slots`.
 *
 * Every trailing argument of a read is a pointer whatever the parameter's type,
 * so only the count matters here. */
static void gather_pointers(va_list ap, int count, void *slots[4])
{
    for (int i = 0; i < count && i < 4; ++i) {
        slots[i] = va_arg(ap, void *);
    }
}

int lumit_ofx_shim_param_get_value(void *param, ...)
{
    /* Filled in only as far as the arity says. When Rust cannot answer — a push
     * button, a descriptor's parameter, a handle that names nothing — the loop
     * is skipped and the call goes through with four empty slots, so the
     * refusal is the sentence Rust has always given rather than a new one. */
    void *slots[4] = { NULL, NULL, NULL, NULL };
    int count = 0;
    int reading = 0;

    if (lumit_ofx_param_arity(param, &count, &reading) == 0) {
        va_list ap;
        va_start(ap, param);
        gather_pointers(ap, count, slots);
        va_end(ap);
    }
    return lumit_ofx_param_get_value(param, slots[0], slots[1], slots[2], slots[3]);
}

int lumit_ofx_shim_param_get_value_at_time(void *param, double time, ...)
{
    void *slots[4] = { NULL, NULL, NULL, NULL };
    int count = 0;
    int reading = 0;

    if (lumit_ofx_param_arity(param, &count, &reading) == 0) {
        va_list ap;
        /* The last *named* argument is `time`, not `param`. Naming `param` here
         * would make the first value pulled the time itself, and no compiler
         * would say a word about it. */
        va_start(ap, time);
        gather_pointers(ap, count, slots);
        va_end(ap);
    }
    return lumit_ofx_param_get_value_at_time(param, time, slots[0], slots[1], slots[2],
                                             slots[3]);
}

/* ---------------------------------------------------------- writing values -- */

/* Pull `count` values into `words`, each as the machine word Rust reads back.
 *
 * Unlike a read these are not all pointers: an `int`, a `double` or a
 * `const char *` depending on what the parameter was defined as, which is why
 * the reading has to be known before the walk starts. */
static void gather_words(va_list ap, int count, int reading, uint64_t words[4])
{
    for (int i = 0; i < count && i < 4; ++i) {
        switch (reading) {
        case LUMIT_OFX_READING_INT:
            /* A bool and a choice arrive as `int` too: C promotes a narrower
             * type before pushing it as a variadic argument. */
            words[i] = (uint64_t)(uint32_t)va_arg(ap, int);
            break;
        case LUMIT_OFX_READING_DOUBLE: {
            double value = va_arg(ap, double);
            /* `memcpy`, not a cast: what crosses is the double's *bits*, which
             * Rust reads back with `f64::from_bits`. A cast would round the
             * number to an integer on the way over. */
            memcpy(&words[i], &value, sizeof value);
            break;
        }
        case LUMIT_OFX_READING_TEXT:
        default:
            words[i] = (uint64_t)(uintptr_t)va_arg(ap, const char *);
            break;
        }
    }
}

int lumit_ofx_shim_param_set_value(void *param, ...)
{
    uint64_t words[4] = { 0, 0, 0, 0 };
    int count = 0;
    int reading = 0;

    if (lumit_ofx_param_arity(param, &count, &reading) == 0) {
        va_list ap;
        va_start(ap, param);
        gather_words(ap, count, reading, words);
        va_end(ap);
    }
    return lumit_ofx_param_set_value(param, words[0], words[1], words[2], words[3]);
}

int lumit_ofx_shim_param_set_value_at_time(void *param, double time, ...)
{
    uint64_t words[4] = { 0, 0, 0, 0 };
    int count = 0;
    int reading = 0;

    if (lumit_ofx_param_arity(param, &count, &reading) == 0) {
        va_list ap;
        /* `time`, as above. */
        va_start(ap, time);
        gather_words(ap, count, reading, words);
        va_end(ap);
    }
    return lumit_ofx_param_set_value_at_time(param, time, words[0], words[1], words[2],
                                             words[3]);
}
