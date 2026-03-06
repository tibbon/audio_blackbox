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

/* Error codes returned by blackbox_* functions.
 * Success is 0; all errors are negative. Retrieve the human-readable
 * message for any non-zero code with blackbox_get_last_error().
 */
#define BLACKBOX_OK                  0
#define BLACKBOX_ERR_INVALID_HANDLE -1
#define BLACKBOX_ERR_AUDIO_DEVICE   -2
#define BLACKBOX_ERR_CONFIG         -3
#define BLACKBOX_ERR_IO             -4
#define BLACKBOX_ERR_LOCK_POISONED  -5
#define BLACKBOX_ERR_INTERNAL       -6

/* Opaque handle to the Rust audio engine. */
typedef struct BlackboxHandle BlackboxHandle;

/*
 * Lightweight status flags for the 1 Hz polling loop.
 * No JSON, no string allocation — just plain C fields.
 */
typedef struct {
    uint64_t write_errors;
    uint32_t sample_rate;
    bool is_recording;
    bool gate_idle;
    bool disk_space_low;
    bool stream_error;
    bool sample_rate_changed;
} StatusFlags;

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
 * Returns BLACKBOX_OK on success, or a negative error code.
 */
int32_t blackbox_start_recording(BlackboxHandle *handle);

/*
 * Stop recording.
 * Returns BLACKBOX_OK on success, or a negative error code.
 */
int32_t blackbox_stop_recording(BlackboxHandle *handle);

/*
 * Check whether recording is currently active.
 */
bool blackbox_is_recording(const BlackboxHandle *handle);

/*
 * Fill a StatusFlags struct with current engine status.
 * Zero-allocation, no JSON — designed for the 1 Hz polling loop.
 * Returns BLACKBOX_OK on success, or a negative error code.
 */
int32_t blackbox_get_status_flags(const BlackboxHandle *handle, StatusFlags *out);

/*
 * Return a JSON array of available input device names.
 * Example: ["MacBook Pro Microphone", "External USB Mic"]
 * Caller must free the returned string with blackbox_free_string().
 * Returns NULL on failure.
 */
char *blackbox_list_input_devices(void);

/*
 * Get the input channel count for a device by name.
 * Pass NULL or "" for the system default device.
 * Returns the channel count (>= 1), or BLACKBOX_ERR_AUDIO_DEVICE on error.
 */
int32_t blackbox_get_device_channel_count(const char *device_name);

/*
 * Update configuration from a JSON string.
 * Only fields present in the JSON are updated; others are left unchanged.
 * Returns BLACKBOX_OK on success, or a negative error code.
 */
int32_t blackbox_set_config_json(BlackboxHandle *handle, const char *json);

/*
 * Write current peak levels into a caller-provided float buffer.
 * out must point to an array of at least max_channels floats.
 * Returns the number of channels written, or a negative error code.
 * Lightweight zero-allocation read for meter UIs.
 */
int32_t blackbox_get_peak_levels(const BlackboxHandle *handle, float *out, int32_t max_channels);

/*
 * Start audio monitoring (peak levels without recording to disk).
 * Returns BLACKBOX_OK on success, or a negative error code.
 */
int32_t blackbox_start_monitoring(BlackboxHandle *handle);

/*
 * Stop audio monitoring.
 * Returns BLACKBOX_OK on success, or a negative error code.
 */
int32_t blackbox_stop_monitoring(BlackboxHandle *handle);

/*
 * Check whether audio monitoring is currently active.
 */
bool blackbox_is_monitoring(const BlackboxHandle *handle);

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
