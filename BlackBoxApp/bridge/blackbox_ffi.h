/*
 * blackbox_ffi.h — C header for the BlackBox Audio Recorder FFI layer.
 *
 * Import this header in your Swift bridging module to call the Rust audio engine.
 * All returned strings must be freed with blackbox_free_string().
 */

#ifndef BLACKBOX_FFI_H
#define BLACKBOX_FFI_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle to the Rust audio engine. */
typedef struct BlackboxHandle BlackboxHandle;

/*
 * Create a new handle from a JSON configuration string.
 * Pass NULL or "" for default configuration.
 * Returns NULL on failure (should not happen with defaults).
 */
BlackboxHandle *blackbox_create(const char *config_json);

/*
 * Destroy a handle, freeing all resources.
 * Stops recording if active. Passing NULL is a safe no-op.
 */
void blackbox_destroy(BlackboxHandle *handle);

/*
 * Start recording with the current configuration.
 * Returns 0 on success, -1 on error.
 */
int32_t blackbox_start_recording(BlackboxHandle *handle);

/*
 * Stop recording.
 * Returns 0 on success, -1 on error.
 */
int32_t blackbox_stop_recording(BlackboxHandle *handle);

/*
 * Check whether recording is currently active.
 */
bool blackbox_is_recording(const BlackboxHandle *handle);

/*
 * Return a JSON object with the current status.
 * Example: {"recording": true, "input_device": "MacBook Pro Microphone"}
 * Caller must free the returned string with blackbox_free_string().
 * Returns NULL on failure.
 */
char *blackbox_get_status_json(const BlackboxHandle *handle);

/*
 * Return a JSON array of available input device names.
 * Example: ["MacBook Pro Microphone", "External USB Mic"]
 * Caller must free the returned string with blackbox_free_string().
 * Returns NULL on failure.
 */
char *blackbox_list_input_devices(void);

/*
 * Update configuration from a JSON string.
 * Only fields present in the JSON are updated; others are left unchanged.
 * Returns 0 on success, -1 on error.
 */
int32_t blackbox_set_config_json(BlackboxHandle *handle, const char *json);

/*
 * Write current peak levels into a caller-provided float buffer.
 * out must point to an array of at least max_channels floats.
 * Returns the number of channels written, or -1 on error.
 * Lightweight alternative to blackbox_get_status_json for meter UIs.
 */
int32_t blackbox_get_peak_levels(const BlackboxHandle *handle, float *out, int32_t max_channels);

/*
 * Return the current configuration as a JSON string.
 * Caller must free the returned string with blackbox_free_string().
 * Returns NULL on failure.
 */
char *blackbox_get_config_json(const BlackboxHandle *handle);

/*
 * Get the last error message, or NULL if no error has occurred.
 * Caller must free the returned string with blackbox_free_string().
 */
char *blackbox_get_last_error(const BlackboxHandle *handle);

/*
 * Free a string previously returned by any blackbox_* function.
 * Passing NULL is a safe no-op.
 */
void blackbox_free_string(char *s);

#ifdef __cplusplus
}
#endif

#endif /* BLACKBOX_FFI_H */
