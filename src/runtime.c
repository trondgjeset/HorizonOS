#include <stddef.h>
#include <stdint.h>

/*
 * Minimal freestanding compiler-runtime support. These functions run after
 * boot.s has established the stack; CPU tables and privilege setup live in Rust.
 */
void *memcpy(void *destination, const void *source, size_t count) {
    unsigned char *out = destination;
    const unsigned char *in = source;

    for (size_t index = 0; index < count; ++index) {
        out[index] = in[index];
    }
    return destination;
}

void *memmove(void *destination, const void *source, size_t count) {
    unsigned char *out = destination;
    const unsigned char *in = source;

    /* Copy forward when ranges do not overlap in the dangerous direction;
       otherwise walk backward so source bytes are not overwritten early. */
    if ((uintptr_t)out < (uintptr_t)in) {
        for (size_t index = 0; index < count; ++index) {
            out[index] = in[index];
        }
    } else if ((uintptr_t)out > (uintptr_t)in) {
        while (count > 0) {
            --count;
            out[count] = in[count];
        }
    }
    return destination;
}

void *memset(void *destination, int value, size_t count) {
    unsigned char *out = destination;

    for (size_t index = 0; index < count; ++index) {
        out[index] = (unsigned char)value;
    }
    return destination;
}

int memcmp(const void *left, const void *right, size_t count) {
    const unsigned char *a = left;
    const unsigned char *b = right;

    for (size_t index = 0; index < count; ++index) {
        if (a[index] != b[index]) {
            return (int)a[index] - (int)b[index];
        }
    }
    return 0;
}

int bcmp(const void *left, const void *right, size_t count) {
    return memcmp(left, right, count);
}

size_t strlen(const char *text) {
    size_t length = 0;
    while (text[length] != '\0') {
        ++length;
    }
    return length;
}

_Noreturn void _Unwind_Resume(void *exception) {
    (void)exception;
    /* The kernel is panic=abort; reaching unwind resume is an unsupported path. */
    __builtin_trap();
}

void rust_eh_personality(void) {}
