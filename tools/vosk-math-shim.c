/* Vosk math shim for webOS.
 *
 * The pre-built ARMv7 libvosk.so was compiled against glibc 2.27 and
 * references versioned float-math symbols such as `powf@GLIBC_2.27`.
 * The webOS SDK sysroot ships an older glibc whose libm does not export
 * those specific symbol versions, so the static linker cannot resolve them.
 *
 * We provide unversioned definitions of the float-math functions here.
 * They delegate to the double-precision libm functions (which exist at the
 * older versions webOS provides). An unversioned definition in the main
 * executable satisfies libvosk's versioned references at both link time and
 * at runtime.
 *
 * Compile with -fno-builtin so calls like (float)sqrt((double)x) are not
 * folded back into sqrtf and cause infinite recursion.
 */
#include <math.h>

float powf(float x, float y)    { return (float)pow((double)x, (double)y); }
float expf(float x)             { return (float)exp((double)x); }
float exp2f(float x)            { return (float)exp2((double)x); }
float expm1f(float x)           { return (float)expm1((double)x); }
float logf(float x)             { return (float)log((double)x); }
float log2f(float x)            { return (float)log2((double)x); }
float log10f(float x)           { return (float)log10((double)x); }
float log1pf(float x)           { return (float)log1p((double)x); }
float sqrtf(float x)            { return (float)sqrt((double)x); }
float cbrtf(float x)            { return (float)cbrt((double)x); }
float sinf(float x)             { return (float)sin((double)x); }
float cosf(float x)             { return (float)cos((double)x); }
float tanf(float x)             { return (float)tan((double)x); }
float asinf(float x)            { return (float)asin((double)x); }
float acosf(float x)            { return (float)acos((double)x); }
float atanf(float x)            { return (float)atan((double)x); }
float atan2f(float y, float x)  { return (float)atan2((double)y, (double)x); }
float sinhf(float x)            { return (float)sinh((double)x); }
float coshf(float x)            { return (float)cosh((double)x); }
float tanhf(float x)            { return (float)tanh((double)x); }
float fmodf(float x, float y)   { return (float)fmod((double)x, (double)y); }
float hypotf(float x, float y)  { return (float)hypot((double)x, (double)y); }
float ldexpf(float x, int e)    { return (float)ldexp((double)x, e); }
float frexpf(float x, int *e)   { return (float)frexp((double)x, e); }

float modff(float x, float *ip)
{
    double i;
    float frac = (float)modf((double)x, &i);
    *ip = (float)i;
    return frac;
}

float fabsf(float x)            { return x < 0.0f ? -x : x; }
float copysignf(float x, float y) { return y < 0.0f ? -fabsf(x) : fabsf(x); }
float floorf(float x)           { return (float)floor((double)x); }
float ceilf(float x)            { return (float)ceil((double)x); }
float truncf(float x)           { return (float)trunc((double)x); }
float roundf(float x)           { return (float)round((double)x); }
float fmaxf(float x, float y)   { return x > y ? x : y; }
float fminf(float x, float y)   { return x < y ? x : y; }
