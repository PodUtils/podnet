/*
 * The C test of libpodnet (T-304). It makes two installs, joins two ends
 * over a memory carrier, opens a stream, echoes 1 MiB through the peer and
 * compares the digest of the bytes that it sent with the digest of the
 * bytes that came back.
 *
 *   cc -I include tests/c_abi.c -L DIR -lpodnet_capi -lpthread -o c_abi
 *   ./c_abi WORKDIR
 *
 * It exits 0 when each check passes, and 1 with one line otherwise.
 */
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

#include "podnet.h"

#define TOTAL (1u << 20)
#define CHUNK 4096

static void die(const char *what, int code)
{
    fprintf(stderr, "c_abi: %s: %d: %s\n", what, code, podnet_last_error());
    exit(1);
}

#define CHECK(what, call)                \
    do {                                 \
        int rc_ = (call);                \
        if (rc_ < 0) die(what, rc_);     \
    } while (0)

static uint64_t fnv(uint64_t h, const unsigned char *p, size_t n)
{
    for (size_t i = 0; i < n; i++) {
        h ^= p[i];
        h *= 1099511628211ULL;
    }
    return h;
}

static void pattern(unsigned char *p, size_t n)
{
    uint32_t x = 12345;
    for (size_t i = 0; i < n; i++) {
        x = x * 1664525u + 1013904223u;
        p[i] = (unsigned char)(x >> 24);
    }
}

/* The listener: accept one stream, check its service and echo until the end. */
static void *echo(void *arg)
{
    podnet_link *l = arg;
    podnet_stream *s = NULL;
    unsigned char buf[CHUNK * 4];
    CHECK("accept", podnet_link_accept(l, &s));
    if (strcmp(podnet_stream_service(s), "echo") != 0) die("service name", 0);
    size_t seen = 0;
    while (seen < TOTAL) {
        int n = podnet_stream_read(s, buf, sizeof buf);
        if (n < 0) die("echo read", n);
        if (n == 0) break;
        CHECK("echo write", podnet_stream_write(s, buf, (size_t)n));
        seen += (size_t)n;
    }
    /* The close ends the stream: the other end reads 0. */
    CHECK("echo close", podnet_stream_close(s));
    return NULL;
}

static void *collect(void *arg)
{
    void **a = arg;
    podnet_stream *s = a[0];
    uint64_t *digest = a[1];
    unsigned char buf[CHUNK * 3];
    size_t got = 0;
    while (got < TOTAL) {
        int n = podnet_stream_read(s, buf, sizeof buf);
        if (n < 0) die("read", n);
        if (n == 0) break;
        *digest = fnv(*digest, buf, (size_t)n);
        got += (size_t)n;
    }
    if (got != TOTAL) {
        fprintf(stderr, "c_abi: read %zu bytes, expected %u\n", got, TOTAL);
        exit(1);
    }
    return NULL;
}

int main(int argc, char **argv)
{
    char dirs[2][512], fp[2][PODNET_FP_MAX];
    const char *err = NULL;
    if (argc != 2) {
        fprintf(stderr, "usage: c_abi WORKDIR\n");
        return 2;
    }
    if (podnet_version() == NULL || *podnet_version() == 0) die("version", 0);
    if (podnet_init(NULL, &err) != 0) die("init", -1);
    CHECK("init again", podnet_init("{\"threads\": 2}", &err));
    if (*podnet_last_error() != 0) die("last_error after success", 0);
    for (int i = 0; i < 2; i++) {
        snprintf(dirs[i], sizeof dirs[i], "%s/id%d", argv[1], i);
        mkdir(dirs[i], 0700);
        CHECK("fingerprint", podnet_identity_fingerprint(dirs[i], fp[i], sizeof fp[i]));
    }
    if (podnet_identity_fingerprint(dirs[0], fp[0], 4) != PODNET_E_LIMIT) die("small buffer", 0);

    /* A wrong fingerprint is refused with PODNET_E_PERMISSION. */
    {
        podnet_link *a = NULL, *b = NULL;
        CHECK("new a", podnet_link_new(dirs[0], fp[0], &a)); /* a wants itself */
        CHECK("new b", podnet_link_new(dirs[1], fp[0], &b));
        if (podnet_link_pair(a, b) != PODNET_E_PERMISSION) die("refused key", 0);
        CHECK("close a", podnet_link_close(a));
        CHECK("close b", podnet_link_close(b));
    }

    podnet_link *a = NULL, *b = NULL;
    CHECK("new a", podnet_link_new(dirs[0], fp[1], &a));
    CHECK("new b", podnet_link_new(dirs[1], fp[0], &b));
    CHECK("pair", podnet_link_pair(a, b));
    if (podnet_link_pair(a, b) != PODNET_E_STATE) die("pair twice", 0);

    pthread_t server, reader;
    if (pthread_create(&server, NULL, echo, b) != 0) return 1;
    podnet_stream *s = NULL;
    CHECK("open", podnet_link_open(a, "echo", &s));

    uint64_t back = 14695981039346656037ULL, sent = back;
    void *args[2] = {s, &back};
    if (pthread_create(&reader, NULL, collect, args) != 0) return 1;
    unsigned char *data = malloc(TOTAL);
    if (data == NULL) return 1;
    pattern(data, TOTAL);
    for (size_t off = 0; off < TOTAL; off += CHUNK * 2) {
        CHECK("write", podnet_stream_write(s, data + off, CHUNK * 2));
    }
    sent = fnv(sent, data, TOTAL);
    pthread_join(server, NULL);
    pthread_join(reader, NULL);
    /* The echo closed its end: this end reads the end of the stream. */
    {
        unsigned char one[1];
        int n = podnet_stream_read(s, one, sizeof one);
        if (n != 0) die("end of stream", n);
    }
    free(data);
    if (sent != back) {
        fprintf(stderr, "c_abi: digest differs: %016llx %016llx\n",
                (unsigned long long)sent, (unsigned long long)back);
        return 1;
    }
    CHECK("close stream", podnet_stream_close(s));
    CHECK("close a", podnet_link_close(a));
    CHECK("close b", podnet_link_close(b));
    CHECK("free", podnet_free());
    printf("c_abi: ok, 1 MiB echoed, digest %016llx\n", (unsigned long long)sent);
    return 0;
}
