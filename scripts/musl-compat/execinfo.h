#pragma once

/*
 * ONNX Runtime 1.20 includes execinfo.h on every non-Android POSIX build,
 * even though its backtrace implementation is compiled out in Release mode.
 * musl intentionally does not provide this glibc extension, so declarations
 * are sufficient for the static Release build performed by our script.
 */

#ifdef __cplusplus
extern "C" {
#endif

int backtrace(void **buffer, int size);
char **backtrace_symbols(void *const *buffer, int size);
void backtrace_symbols_fd(void *const *buffer, int size, int fd);

#ifdef __cplusplus
}
#endif
