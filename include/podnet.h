/*
 * podnet.h: the C interface of the podnet link (T-304).
 *
 * Library: libpodnet.so (shared) and libpodnet.a (static). Build them with
 * crates/podnet-capi (docs/build.md, section The C library).
 *
 * Rules:
 * - A function that returns int returns 0 or a positive count on success.
 *   It returns a negative PODNET_E_* code on failure.
 * - podnet_last_error() returns the text of the last failure of the
 *   calling thread. The text is UTF-8, NUL-terminated and at most 64 KiB.
 *   It stays valid until the next podnet call on the same thread. Each
 *   podnet call but podnet_last_error() and podnet_version() clears it.
 * - A string argument is UTF-8 and NUL-terminated. The caller owns each
 *   buffer. The library never frees a pointer of the caller, and the
 *   caller never frees a pointer of the library: a handle ends with its
 *   close function.
 * - A handle is opaque. A handle that was closed must not be used again.
 * - No Rust panic crosses this interface. A panic is PODNET_E_PANIC.
 * - The library calls no function of the caller.
 * - podnet_stream_read() and podnet_stream_write() block. Use one thread
 *   for each direction of a stream. Do not close a stream or a link while
 *   another thread uses it.
 * - A call that takes a byte count takes at most PODNET_MAX_IO bytes.
 */
#ifndef PODNET_H
#define PODNET_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Codes. */
#define PODNET_OK 0
#define PODNET_E_ARG (-1)         /* a bad argument: NULL, no UTF-8, out of range */
#define PODNET_E_UNAVAILABLE (-2) /* the peer, the service or the carrier is not there */
#define PODNET_E_PERMISSION (-3)  /* a peer key was refused */
#define PODNET_E_IO (-4)          /* a failure of the link or of the host */
#define PODNET_E_STATE (-5)       /* a call that does not fit the state of the handle */
#define PODNET_E_LIMIT (-6)       /* a size over a limit */
#define PODNET_E_PANIC (-99)      /* a panic inside the library */

/* The most bytes of one podnet_stream_read() or podnet_stream_write(). */
#define PODNET_MAX_IO 1048576
/* The size of a buffer that always holds a fingerprint, with the NUL. */
#define PODNET_FP_MAX 128
/* The most bytes of the text of podnet_last_error(), with the NUL. */
#define PODNET_MAX_ERROR 65536

typedef struct podnet_link podnet_link;
typedef struct podnet_stream podnet_stream;

/*
 * Start the library. config_json is NULL, "" or a JSON object:
 * {"threads": N} sets the worker threads (1 to 64, default 2). Another
 * call after a success returns 0 and changes nothing. A call after
 * podnet_free() starts the library again. The other functions start it with
 * the default configuration when no call did. On failure and when err is
 * not NULL, *err is the text of podnet_last_error().
 */
int podnet_init(const char *config_json, const char **err);

/*
 * Stop the library. It returns PODNET_E_STATE and stops nothing while a
 * link or a stream is open.
 */
int podnet_free(void);

/*
 * Write the fingerprint of the install key in the directory identity_path
 * to buf (NUL-terminated, at most PODNET_FP_MAX bytes with the NUL). The
 * library makes the key when none exists. A peer gives this text as its
 * peer_fp to the other end. It returns PODNET_E_LIMIT when len is too small.
 */
int podnet_identity_fingerprint(const char *identity_path, char *buf, size_t len);

/*
 * Make an end of a link. identity_path is the directory of the install key:
 * the library makes the key there when none exists. peer_fp is the one
 * fingerprint that the peer must prove. The end has no peer until
 * podnet_link_pair().
 */
int podnet_link_new(const char *identity_path, const char *peer_fp, podnet_link **out);

/*
 * Join two ends over a memory carrier with a ring of 1 MiB in each
 * direction, and run the hello of each end (each key proves itself and
 * must be the peer_fp of the other end). It blocks until the hello ends.
 * dialer opens streams first by convention; both ends can open and accept.
 * A key that is not the peer_fp returns PODNET_E_PERMISSION. Each end can be
 * joined once, otherwise PODNET_E_STATE.
 */
int podnet_link_pair(podnet_link *dialer, podnet_link *listener);

/*
 * Open a stream to the service of the peer. service is 1 to 32 characters
 * from A-Z a-z 0-9 . _ - . It blocks until the peer answers.
 */
int podnet_link_open(podnet_link *l, const char *service, podnet_stream **out);

/*
 * Wait for a stream that the peer opened, and accept it. It blocks.
 */
int podnet_link_accept(podnet_link *l, podnet_stream **out);

/* End the link, and free the handle. The streams of the link end. */
int podnet_link_close(podnet_link *l);

/*
 * The service name of an accepted or opened stream. The text is valid
 * until podnet_stream_close(). It is NULL for a NULL stream.
 */
const char *podnet_stream_service(const podnet_stream *s);

/*
 * Read at most len bytes (1 to PODNET_MAX_IO) into buf. It blocks until at
 * least one byte is there. It returns the count, or 0 at the end of the
 * stream.
 */
int podnet_stream_read(podnet_stream *s, void *buf, size_t len);

/*
 * Write len bytes (0 to PODNET_MAX_IO) from buf. It blocks until the link
 * took all of them. It returns len.
 */
int podnet_stream_write(podnet_stream *s, const void *buf, size_t len);

/*
 * End the writing of the stream (the peer reads the end of the stream),
 * and free the handle.
 */
int podnet_stream_close(podnet_stream *s);

/* The text of the last failure of this thread, "" when none. Never NULL. */
const char *podnet_last_error(void);

/* The version of the library, for example "0.4.0-beta.9". Never NULL. */
const char *podnet_version(void);

#ifdef __cplusplus
}
#endif

#endif /* PODNET_H */
