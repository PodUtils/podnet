/*
 * libpodnet-vnet.so — map AF_INET/AF_INET6 sockets into a userspace virtual
 * network by rewriting them to AF_UNIX (abstract) sockets.
 *
 * The kernel never sees an AF_INET bind(2), connect(2) or listen(2). Each IP
 * endpoint becomes a name in the abstract namespace:
 *
 *     \0podnet-vnet/<family>/<address>/<port>
 *
 * An IPv4-mapped IPv6 address gets the IPv4 name.
 *
 * Two interposed programs on the same host therefore talk over AF_UNIX with
 * no host process at all ("direct mode"). When PODNET_VNET_CONTROL names the
 * stack's control socket, bind(2) also registers the endpoint so the stack
 * can bridge it to a tunnel.
 *
 * PODNET_VNET_SOCKS names the unix socket of a SOCKS5 front door, and
 * PODNET_VNET_SOCKS_ROUTES a comma list of destinations (tailnet, ADDR,
 * ADDR/BITS, IPV4:PORT or [IPV6]:PORT). A stream connect to a listed
 * destination goes to that door, not to an abstract name.
 *
 * PODNET_VNET_FORWARD is a comma list of ADDR:PORT=unix:/PATH. A stream
 * connect to a listed destination goes to the unix socket PATH. The
 * forwards come first, then the SOCKS routes, then the abstract name.
 *
 * socket(AF_NETLINK, *, NETLINK_ROUTE), getifaddrs and if_nameindex give the
 * interfaces of the virtual network: lo and vnet0 with the local address
 * (T-392). PODNET_VNET_HOST_NETLINK=1 gives them to the host.
 *
 * This file is compiled from the embedded copy in crates/podnet-vnet/src/shim.rs, so a
 * podnet binary can build it anywhere. It is deliberately C: interposition has
 * to happen at the libc symbol boundary, before any Rust constructor runs.
 */
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <ifaddrs.h>
#include <linux/netlink.h>
#include <linux/rtnetlink.h>
#include <net/if.h>
#include <net/if_arp.h>
#include <netdb.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <pthread.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

/* ── real symbols ─────────────────────────────────────────────────────────── */

static int (*r_socket)(int, int, int);
static int (*r_socketpair)(int, int, int, int[2]);
static int (*r_bind)(int, const struct sockaddr *, socklen_t);
static int (*r_connect)(int, const struct sockaddr *, socklen_t);
static int (*r_listen)(int, int);
static int (*r_accept)(int, struct sockaddr *, socklen_t *);
static int (*r_accept4)(int, struct sockaddr *, socklen_t *, int);
static int (*r_getpeername)(int, struct sockaddr *, socklen_t *);
static int (*r_getsockname)(int, struct sockaddr *, socklen_t *);
static int (*r_close)(int);
static int (*r_dup)(int);
static int (*r_dup2)(int, int);
static int (*r_dup3)(int, int, int);
static int (*r_fcntl)(int, int, ...);
static ssize_t (*r_sendto)(int, const void *, size_t, int, const struct sockaddr *, socklen_t);
static ssize_t (*r_recvfrom)(int, void *, size_t, int, struct sockaddr *, socklen_t *);
static ssize_t (*r_sendmsg)(int, const struct msghdr *, int);
static ssize_t (*r_recvmsg)(int, struct msghdr *, int);
static int (*r_setsockopt)(int, int, int, const void *, socklen_t);
static int (*r_getsockopt)(int, int, int, void *, socklen_t *);
static int (*r_getaddrinfo)(const char *, const char *, const struct addrinfo *, struct addrinfo **);
static ssize_t (*r_send)(int, const void *, size_t, int);
static ssize_t (*r_recv)(int, void *, size_t, int);
static int (*r_getifaddrs)(struct ifaddrs **);
static void (*r_freeifaddrs)(struct ifaddrs *);
static unsigned int (*r_if_nametoindex)(const char *);
static char *(*r_if_indextoname)(unsigned int, char *);
static struct if_nameindex *(*r_if_nameindex)(void);
static void (*r_if_freenameindex)(struct if_nameindex *);

/* ── configuration ────────────────────────────────────────────────────────── */

struct config {
    char prefix[64];       /* abstract-name prefix, default "podnet-vnet" */
    char control[108];     /* control socket name without NUL, empty = direct */
    char socks[108];       /* PODNET_VNET_SOCKS: the front door, empty = none */
    int log;
    int map_loopback;      /* rewrite loopback endpoints to local_addr */
    int host_netlink;      /* PODNET_VNET_HOST_NETLINK: the host answers netlink */
    struct in_addr local_v4;   /* PODNET_VNET_LOCAL_ADDR, default 10.66.0.2 */
    int max_fds;
    /* hosts table: name -> address, for getaddrinfo interception */
    struct hostent_override { char name[256]; char addr[64]; } hosts[64];
    size_t hosts_len;
};

static struct config cfg;
static pthread_once_t init_once = PTHREAD_ONCE_INIT;
static pthread_once_t cfg_once = PTHREAD_ONCE_INIT;

static void parse_config(void);

static void cfg_init(void) {
    parse_config();
}

static void logf_(const char *fmt, ...) {
    if (!cfg.log) return;
    va_list ap;
    va_start(ap, fmt);
    fputs("[podnet-vnet] ", stderr);
    vfprintf(stderr, fmt, ap);
    va_end(ap);
}

/* ── routes to the SOCKS5 front door ──────────────────────────────── */

/* The same table is in podnet-socks.c and podnet-vnet.c (each shim is one
   file); crates/podnet-vnet/src/shim.rs parse_socks_route checks the same
   forms. */
#define MAX_ROUTES 32

struct route {
    int family;              /* AF_INET or AF_INET6 */
    unsigned char addr[16];
    int bits;                /* prefix length */
    unsigned port;           /* 0: each port */
};

static struct route routes[MAX_ROUTES];
static int routes_len = -1;  /* -1: no list was given */

static void route_add(int family, const unsigned char *addr, long bits, long port,
                      const char *item) {
    if (routes_len >= MAX_ROUTES) {
        logf_("route %s: the table is full (%d routes); it is ignored\n", item, MAX_ROUTES);
        return;
    }
    struct route *r = &routes[routes_len++];
    memset(r, 0, sizeof *r);
    r->family = family;
    memcpy(r->addr, addr, family == AF_INET ? 4 : 16);
    r->bits = (int)bits;
    r->port = (unsigned)port;
    logf_("route %s\n", item);
}

static int parse_number(const char *text, long min, long max, long *out) {
    char *stop;
    if (!*text) return -1;
    long n = strtol(text, &stop, 10);
    if (*stop || n < min || n > max) return -1;
    *out = n;
    return 0;
}

/* One item: tailnet, ADDR, ADDR/BITS, IPV4:PORT or [IPV6]:PORT. */
static void route_parse(const char *item) {
    char host[64];
    unsigned char addr[16];
    long bits = -1, port = 0;
    char *mark;
    if (strcmp(item, "tailnet") == 0) {
        const unsigned char v4[4] = {100, 64, 0, 0};
        const unsigned char v6[16] = {0xfd, 0x7a, 0x11, 0x5c, 0xa1, 0xe0};
        route_add(AF_INET, v4, 10, 0, "tailnet 100.64.0.0/10");
        route_add(AF_INET6, v6, 48, 0, "tailnet fd7a:115c:a1e0::/48");
        return;
    }
    if (strlen(item) >= sizeof host) goto bad;
    if (item[0] == '[') {
        const char *end = strchr(item, ']');
        if (!end) goto bad;
        memcpy(host, item + 1, (size_t)(end - item - 1));
        host[end - item - 1] = '\0';
        if (end[1] == ':') {
            if (parse_number(end + 2, 1, 65535, &port) < 0) goto bad;
        } else if (end[1]) {
            goto bad;
        }
        if (inet_pton(AF_INET6, host, addr) != 1) goto bad;
        route_add(AF_INET6, addr, 128, port, item);
        return;
    }
    snprintf(host, sizeof host, "%s", item);
    if ((mark = strchr(host, '/'))) {
        *mark = '\0';
        if (parse_number(mark + 1, 0, 128, &bits) < 0) goto bad;
    } else if ((mark = strchr(host, ':')) && !strchr(mark + 1, ':')) {
        *mark = '\0';
        if (parse_number(mark + 1, 1, 65535, &port) < 0) goto bad;
    }
    if (inet_pton(AF_INET, host, addr) == 1) {
        if (bits > 32) goto bad;
        route_add(AF_INET, addr, bits < 0 ? 32 : bits, port, item);
        return;
    }
    if (port == 0 && inet_pton(AF_INET6, host, addr) == 1) {
        route_add(AF_INET6, addr, bits < 0 ? 128 : bits, 0, item);
        return;
    }
bad:
    logf_("route %s: not tailnet, ADDR, ADDR/BITS, IPV4:PORT or [IPV6]:PORT; it is ignored\n",
          item);
}

/* Read a comma list. A list replaces the default, also when no item is
   good. */
static void route_parse_list(const char *list) {
    routes_len = 0;
    char *copy = strdup(list);
    if (!copy) return;
    char *save = NULL;
    for (char *tok = strtok_r(copy, ",", &save); tok; tok = strtok_r(NULL, ",", &save)) {
        while (*tok == ' ') tok++;
        size_t n = strlen(tok);
        while (n && tok[n - 1] == ' ') tok[--n] = '\0';
        if (*tok) route_parse(tok);
    }
    free(copy);
}

static int prefix_match(const unsigned char *a, const unsigned char *b, int bits) {
    int whole = bits / 8, rest = bits % 8;
    if (whole && memcmp(a, b, (size_t)whole) != 0) return 0;
    if (!rest) return 1;
    unsigned char mask = (unsigned char)(0xff << (8 - rest));
    return (a[whole] & mask) == (b[whole] & mask);
}

/* 1 when a route of the table takes sa. An IPv4-mapped IPv6 address is
   matched as IPv4. */
static int route_match(const struct sockaddr *sa) {
    int family;
    const unsigned char *addr;
    unsigned port;
    if (sa->sa_family == AF_INET) {
        const struct sockaddr_in *s = (const struct sockaddr_in *)sa;
        family = AF_INET;
        addr = (const unsigned char *)&s->sin_addr;
        port = ntohs(s->sin_port);
    } else if (sa->sa_family == AF_INET6) {
        const struct sockaddr_in6 *s = (const struct sockaddr_in6 *)sa;
        family = AF_INET6;
        addr = s->sin6_addr.s6_addr;
        port = ntohs(s->sin6_port);
        if (IN6_IS_ADDR_V4MAPPED(&s->sin6_addr)) {
            family = AF_INET;
            addr += 12;
        }
    } else {
        return 0;
    }
    for (int i = 0; i < routes_len; i++) {
        const struct route *r = &routes[i];
        if (r->family != family || (r->port && r->port != port)) continue;
        if (prefix_match(addr, r->addr, r->bits)) return 1;
    }
    return 0;
}

/* ── forwards to a unix socket path ───────────────────────────────── */

/* PODNET_VNET_FORWARD: a comma list of ADDR:PORT=unix:/PATH (IPV4:PORT or
   [IPV6]:PORT). A stream connect to ADDR:PORT connects to the unix socket
   /PATH, with no SOCKS step. crates/podnet-vnet/src/shim.rs parse_forward
   checks the same form. */
#define MAX_FORWARDS 16

struct forward {
    int family;              /* AF_INET or AF_INET6 */
    unsigned char addr[16];
    unsigned port;
    char path[sizeof(((struct sockaddr_un *)0)->sun_path)];
};

static struct forward forwards[MAX_FORWARDS];
static int forwards_len;

static void forward_parse(const char *item) {
    char host[64];
    unsigned char addr[16];
    long port = 0;
    int family;
    const char *eq = strchr(item, '=');
    if (!eq || (size_t)(eq - item) >= sizeof host) goto bad;
    memcpy(host, item, (size_t)(eq - item));
    host[eq - item] = '\0';
    const char *path = eq + 1;
    if (strncmp(path, "unix:", 5) != 0) goto bad;
    path += 5;
    if (path[0] != '/' || strlen(path) >= sizeof forwards[0].path) goto bad;
    char *mark;
    if (host[0] == '[') {
        char *end = strchr(host, ']');
        if (!end || end[1] != ':') goto bad;
        *end = '\0';
        if (parse_number(end + 2, 1, 65535, &port) < 0) goto bad;
        if (inet_pton(AF_INET6, host + 1, addr) != 1) goto bad;
        family = AF_INET6;
        if (IN6_IS_ADDR_V4MAPPED((struct in6_addr *)addr)) {
            /* A mapped address matches its IPv4 form. */
            memmove(addr, addr + 12, 4);
            family = AF_INET;
        }
    } else {
        if (!(mark = strrchr(host, ':'))) goto bad;
        *mark = '\0';
        if (parse_number(mark + 1, 1, 65535, &port) < 0) goto bad;
        if (inet_pton(AF_INET, host, addr) != 1) goto bad;
        family = AF_INET;
    }
    if (forwards_len >= MAX_FORWARDS) {
        logf_("forward %s: the table is full (%d forwards); it is ignored\n", item, MAX_FORWARDS);
        return;
    }
    struct forward *f = &forwards[forwards_len++];
    memset(f, 0, sizeof *f);
    f->family = family;
    memcpy(f->addr, addr, family == AF_INET ? 4 : 16);
    f->port = (unsigned)port;
    snprintf(f->path, sizeof f->path, "%s", path);
    logf_("forward %s\n", item);
    return;
bad:
    logf_("forward %s: not IPV4:PORT=unix:/PATH or [IPV6]:PORT=unix:/PATH (a path of at most %u bytes); "
          "it is ignored\n",
          item, (unsigned)(sizeof forwards[0].path - 1));
}

static void forward_parse_list(const char *list) {
    char *copy = strdup(list);
    if (!copy) return;
    char *save = NULL;
    for (char *tok = strtok_r(copy, ",", &save); tok; tok = strtok_r(NULL, ",", &save)) {
        while (*tok == ' ') tok++;
        size_t n = strlen(tok);
        while (n && tok[n - 1] == ' ') tok[--n] = '\0';
        if (*tok) forward_parse(tok);
    }
    free(copy);
}

/* The forward of sa, or NULL. An IPv4-mapped IPv6 address matches as
   IPv4. */
static const struct forward *forward_match(const struct sockaddr *sa, socklen_t len) {
    int family;
    const unsigned char *addr;
    unsigned port;
    if (!forwards_len || !sa) return NULL;
    if (sa->sa_family == AF_INET && len >= (socklen_t)sizeof(struct sockaddr_in)) {
        const struct sockaddr_in *s = (const struct sockaddr_in *)sa;
        family = AF_INET;
        addr = (const unsigned char *)&s->sin_addr;
        port = ntohs(s->sin_port);
    } else if (sa->sa_family == AF_INET6 && len >= (socklen_t)sizeof(struct sockaddr_in6)) {
        const struct sockaddr_in6 *s = (const struct sockaddr_in6 *)sa;
        family = AF_INET6;
        addr = s->sin6_addr.s6_addr;
        port = ntohs(s->sin6_port);
        if (IN6_IS_ADDR_V4MAPPED(&s->sin6_addr)) {
            family = AF_INET;
            addr += 12;
        }
    } else {
        return NULL;
    }
    for (int i = 0; i < forwards_len; i++) {
        const struct forward *f = &forwards[i];
        if (f->family == family && f->port == port &&
            memcmp(f->addr, addr, family == AF_INET ? 4 : 16) == 0)
            return f;
    }
    return NULL;
}

static int valid_prefix(const char *s) {
    size_t n = strlen(s);
    if (n == 0 || n > 59) return 0;
    for (size_t i = 0; i < n; i++) {
        char c = s[i];
        if (!((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
              (c >= '0' && c <= '9') || c == '-' || c == '_' || c == '.'))
            return 0;
    }
    return 1;
}

static void parse_config(void) {
    const char *v;
    memset(&cfg, 0, sizeof cfg);
    snprintf(cfg.prefix, sizeof cfg.prefix, "podnet-vnet");
    cfg.max_fds = 65536;
    cfg.local_v4.s_addr = htonl((10u << 24) | (66u << 16) | (0u << 8) | 2u);
    const char *bad_prefix = NULL;
    if ((v = getenv("PODNET_VNET_PREFIX")) != NULL) {
        if (valid_prefix(v))
            snprintf(cfg.prefix, sizeof cfg.prefix, "%s", v);
        else
            bad_prefix = v;
    }
    if ((v = getenv("PODNET_VNET_CONTROL")) && *v) {
        snprintf(cfg.control, sizeof cfg.control, "%s", v);
    }
    cfg.log = (v = getenv("PODNET_VNET_LOG")) && *v && *v != '0';
    // Print an ignored prefix unconditionally so direct preload never falls
    // back to the default prefix silently.
    if (bad_prefix) {
        fprintf(stderr,
                "podnet-vnet: PODNET_VNET_PREFIX %s is not 1 to 59 bytes of letters, digits, '.', '-' or '_'; using podnet-vnet\n",
                bad_prefix);
    }
    cfg.map_loopback = (v = getenv("PODNET_VNET_MAP_LOOPBACK")) && *v && *v != '0';
    cfg.host_netlink = (v = getenv("PODNET_VNET_HOST_NETLINK")) && *v && *v != '0';
    if ((v = getenv("PODNET_VNET_LOCAL_ADDR")) && *v) {
        if (inet_pton(AF_INET, v, &cfg.local_v4) != 1) {
            logf_("PODNET_VNET_LOCAL_ADDR %s is not an IPv4 address\n", v);
        }
    }
    if ((v = getenv("PODNET_VNET_MAX_FDS")) && *v) {
        long n = strtol(v, NULL, 10);
        if (n > 0) cfg.max_fds = (int)n;
    }
    forwards_len = 0;
    if ((v = getenv("PODNET_VNET_FORWARD")) && *v) forward_parse_list(v);
    if ((v = getenv("PODNET_VNET_SOCKS")) && *v) {
        snprintf(cfg.socks, sizeof cfg.socks, "%s", v);
        if ((v = getenv("PODNET_VNET_SOCKS_ROUTES")) && *v) route_parse_list(v);
    }
    if ((v = getenv("PODNET_VNET_HOSTS")) && *v) {
        char *copy = strdup(v);
        if (copy) {
            char *save = NULL;
            for (char *tok = strtok_r(copy, ",", &save); tok && cfg.hosts_len < 64;
                 tok = strtok_r(NULL, ",", &save)) {
                char *eq = strchr(tok, '=');
                if (!eq) continue;
                *eq = '\0';
                snprintf(cfg.hosts[cfg.hosts_len].name, 256, "%s", tok);
                snprintf(cfg.hosts[cfg.hosts_len].addr, 64, "%s", eq + 1);
                cfg.hosts_len++;
            }
            free(copy);
        }
    }
}

static void resolve_real(void) {
    r_socket = dlsym(RTLD_NEXT, "socket");
    r_socketpair = dlsym(RTLD_NEXT, "socketpair");
    r_bind = dlsym(RTLD_NEXT, "bind");
    r_connect = dlsym(RTLD_NEXT, "connect");
    r_listen = dlsym(RTLD_NEXT, "listen");
    r_accept = dlsym(RTLD_NEXT, "accept");
    r_accept4 = dlsym(RTLD_NEXT, "accept4");
    r_getpeername = dlsym(RTLD_NEXT, "getpeername");
    r_getsockname = dlsym(RTLD_NEXT, "getsockname");
    r_close = dlsym(RTLD_NEXT, "close");
    r_dup = dlsym(RTLD_NEXT, "dup");
    r_dup2 = dlsym(RTLD_NEXT, "dup2");
    r_dup3 = dlsym(RTLD_NEXT, "dup3");
    r_fcntl = dlsym(RTLD_NEXT, "fcntl");
    r_sendto = dlsym(RTLD_NEXT, "sendto");
    r_recvfrom = dlsym(RTLD_NEXT, "recvfrom");
    r_sendmsg = dlsym(RTLD_NEXT, "sendmsg");
    r_recvmsg = dlsym(RTLD_NEXT, "recvmsg");
    r_setsockopt = dlsym(RTLD_NEXT, "setsockopt");
    r_getsockopt = dlsym(RTLD_NEXT, "getsockopt");
    r_getaddrinfo = dlsym(RTLD_NEXT, "getaddrinfo");
    r_send = dlsym(RTLD_NEXT, "send");
    r_recv = dlsym(RTLD_NEXT, "recv");
    r_getifaddrs = dlsym(RTLD_NEXT, "getifaddrs");
    r_freeifaddrs = dlsym(RTLD_NEXT, "freeifaddrs");
    r_if_nametoindex = dlsym(RTLD_NEXT, "if_nametoindex");
    r_if_indextoname = dlsym(RTLD_NEXT, "if_indextoname");
    r_if_nameindex = dlsym(RTLD_NEXT, "if_nameindex");
    r_if_freenameindex = dlsym(RTLD_NEXT, "if_freenameindex");
}

static void ensure_init(void) {
    pthread_once(&init_once, resolve_real);
    pthread_once(&cfg_once, cfg_init);
}

/* Every interposed function must reach ensure_init before it dereferences an
 * r_* pointer or reads cfg, because both start out zeroed.

 * This is not a formality. close(2), dup(2), dup2(2), getpeername(2) and
 * getsockname(2) are called by programs that never open a socket at all: any
 * program that writes a file closes a descriptor first. Without ensure_init in
 * those functions, r_close is NULL at that point and the process dies with
 * SIGSEGV before it does anything useful. A shim that breaks the majority of
 * programs defeats its own purpose, so the check belongs in all of them. */

/* Copy a logical address into the caller's buffer the way Linux does.

   The Linux contract for getsockname/getpeername/getsockopt is: write at most
   *len bytes, then report the address's true length in *len, and succeed. A
   caller that wants the length asks with a zero-length buffer; a caller with a
   deliberately small buffer gets a truncated address, not an error. The
   reference implementation returned EINVAL for a short buffer and wrote a
   whole sockaddr_storage regardless, which both disagreed with the kernel and
   smashed the caller's stack when the buffer was smaller than 128 bytes.

   Returns 0 on success, -1 with errno set for the caller-error cases. */
static int copy_sockaddr(struct sockaddr *sa, socklen_t *slen,
                         const struct sockaddr_storage *addr, socklen_t addr_len) {
    if (!slen) {
        errno = EINVAL;
        return -1;
    }
    if (!sa) {
        /* Linux allows a null buffer purely to query the length. */
        *slen = addr_len;
        return 0;
    }
    socklen_t capacity = *slen;
    socklen_t copy = addr_len < capacity ? addr_len : capacity;
    if (copy) memcpy(sa, addr, copy);
    *slen = addr_len;
    return 0;
}

/* ── fd state table ───────────────────────────────────────────────────────── */

struct virt_fd {
    unsigned char is_virtual;
    unsigned char family;   /* AF_INET / AF_INET6, 0 when unknown */
    unsigned char is_dgram;
    struct sockaddr_storage local;
    socklen_t local_len;
    struct sockaddr_storage peer;
    socklen_t peer_len;
    int tcp_nodelay;        /* recorded IPPROTO_TCP option */
    int tcp_keepalive;
    int so_reuseaddr;
    unsigned char is_netlink; /* a NETLINK_ROUTE socket that the shim answers */
    int nl_peer;            /* its kernel end */
    uint32_t nl_pid;        /* its port id */
};

static struct virt_fd *fd_table;
static size_t fd_table_len;
static pthread_mutex_t fd_lock = PTHREAD_MUTEX_INITIALIZER;

static int table_ensure(int fd) {
    if (fd < 0 || fd >= cfg.max_fds) return -1;
    size_t want = (size_t)fd + 1;
    if (want > fd_table_len) {
        size_t grow = fd_table_len ? fd_table_len : 256;
        while (grow < want) grow *= 2;
        if (grow > (size_t)cfg.max_fds) grow = (size_t)cfg.max_fds;
        struct virt_fd *next = realloc(fd_table, grow * sizeof *next);
        if (!next) return -1;
        memset(next + fd_table_len, 0, (grow - fd_table_len) * sizeof *next);
        fd_table = next;
        fd_table_len = grow;
    }
    return 0;
}

static int fd_is_virtual(int fd) {
    int result = 0;
    pthread_mutex_lock(&fd_lock);
    if (fd >= 0 && (size_t)fd < fd_table_len) result = fd_table[fd].is_virtual;
    pthread_mutex_unlock(&fd_lock);
    return result;
}

static int fd_family(int fd) {
    int result = 0;
    pthread_mutex_lock(&fd_lock);
    if (fd >= 0 && (size_t)fd < fd_table_len) result = fd_table[fd].family;
    pthread_mutex_unlock(&fd_lock);
    return result;
}

static void fd_mark(int fd, int family, int type) {
    pthread_mutex_lock(&fd_lock);
    if (table_ensure(fd) == 0) {
        fd_table[fd].is_virtual = 1;
        fd_table[fd].family = (unsigned char)family;
        fd_table[fd].is_dgram = ((type & 0xf) == SOCK_DGRAM);
    }
    pthread_mutex_unlock(&fd_lock);
}

static void fd_clear(int fd) {
    pthread_mutex_lock(&fd_lock);
    if (fd >= 0 && (size_t)fd < fd_table_len) memset(&fd_table[fd], 0, sizeof fd_table[fd]);
    pthread_mutex_unlock(&fd_lock);
}

static void fd_copy(int from, int to) {
    pthread_mutex_lock(&fd_lock);
    if (from >= 0 && (size_t)from < fd_table_len && table_ensure(to) == 0) {
        fd_table[to] = fd_table[from];
    }
    pthread_mutex_unlock(&fd_lock);
}

static void fd_record_local(int fd, const struct sockaddr *sa, socklen_t len) {
    pthread_mutex_lock(&fd_lock);
    if (fd >= 0 && (size_t)fd < fd_table_len && len <= sizeof(struct sockaddr_storage)) {
        memcpy(&fd_table[fd].local, sa, len);
        fd_table[fd].local_len = len;
        fd_table[fd].family = (unsigned char)sa->sa_family;
    }
    pthread_mutex_unlock(&fd_lock);
}

static void fd_record_peer(int fd, const struct sockaddr *sa, socklen_t len) {
    pthread_mutex_lock(&fd_lock);
    if (fd >= 0 && (size_t)fd < fd_table_len && len <= sizeof(struct sockaddr_storage)) {
        memcpy(&fd_table[fd].peer, sa, len);
        fd_table[fd].peer_len = len;
    }
    pthread_mutex_unlock(&fd_lock);
}

/* ── address translation ──────────────────────────────────────────────────── */

/* RFC 5952 formatting, matching crates/podnet-vnet/src/addr.rs byte for byte. */
static void format_v6(const unsigned char *addr, char *out, size_t out_len) {
    uint16_t groups[8];
    for (int i = 0; i < 8; i++) groups[i] = (uint16_t)((addr[2 * i] << 8) | addr[2 * i + 1]);
    int best_start = -1, best_len = 0;
    for (int i = 0; i < 8;) {
        if (groups[i] == 0) {
            int start = i;
            while (i < 8 && groups[i] == 0) i++;
            int len = i - start;
            if (len > best_len) {
                best_len = len;
                best_start = start;
            }
        } else {
            i++;
        }
    }
    if (best_len < 2) {
        best_start = -1;
        best_len = 0;
    }
    char buf[64];
    size_t used = 0;
    buf[0] = '\0';
    for (int i = 0; i < 8;) {
        if (i == best_start) {
            used += (size_t)snprintf(buf + used, sizeof buf - used, "::");
            i += best_len;
            continue;
        }
        if (used > 0 && buf[used - 1] != ':') {
            used += (size_t)snprintf(buf + used, sizeof buf - used, ":");
        }
        used += (size_t)snprintf(buf + used, sizeof buf - used, "%x", groups[i]);
        i++;
    }
    if (used == 0) snprintf(buf, sizeof buf, "::");
    snprintf(out, out_len, "%s", buf);
}

/* An IPv4-mapped IPv6 address (::ffff:A.B.C.D) is an IPv4
   endpoint. Give its sockaddr_in in *v4 and 1, or 0 for each other
   address. A dual-mode AF_INET6 socket (.NET) connects to the mapped form
   of 127.0.0.1, and must meet a listener of 127.0.0.1. */
static int unmap_v4(const struct sockaddr *sa, socklen_t len, struct sockaddr_in *v4) {
    if (!sa || sa->sa_family != AF_INET6 || len < (socklen_t)sizeof(struct sockaddr_in6)) return 0;
    const struct sockaddr_in6 *s = (const struct sockaddr_in6 *)sa;
    if (!IN6_IS_ADDR_V4MAPPED(&s->sin6_addr)) return 0;
    memset(v4, 0, sizeof *v4);
    v4->sin_family = AF_INET;
    v4->sin_port = s->sin6_port;
    memcpy(&v4->sin_addr, s->sin6_addr.s6_addr + 12, 4);
    return 1;
}

/* Give an address in the family of the descriptor. An AF_INET6
   descriptor gets an IPv4 address in the mapped form, an AF_INET
   descriptor a mapped address in the IPv4 form. */
static void fit_family(int family, struct sockaddr_storage *ss, socklen_t *len) {
    if (family == AF_INET6 && ss->ss_family == AF_INET) {
        struct sockaddr_in v4 = *(struct sockaddr_in *)ss;
        struct sockaddr_in6 *s6 = (struct sockaddr_in6 *)ss;
        memset(ss, 0, sizeof *ss);
        s6->sin6_family = AF_INET6;
        s6->sin6_port = v4.sin_port;
        s6->sin6_addr.s6_addr[10] = 0xff;
        s6->sin6_addr.s6_addr[11] = 0xff;
        memcpy(s6->sin6_addr.s6_addr + 12, &v4.sin_addr, 4);
        *len = sizeof *s6;
    } else if (family == AF_INET) {
        struct sockaddr_in v4;
        if (unmap_v4((struct sockaddr *)ss, *len, &v4)) {
            memset(ss, 0, sizeof *ss);
            memcpy(ss, &v4, sizeof v4);
            *len = sizeof v4;
        }
    }
}

static int to_abstract(const struct sockaddr *sa, socklen_t len,
                       struct sockaddr_un *un, socklen_t *un_len) {
    char ip[64];
    unsigned port;
    int family;
    struct sockaddr_in v4;
    if (unmap_v4(sa, len, &v4)) {
        sa = (const struct sockaddr *)&v4;
        len = sizeof v4;
    } else if (sa->sa_family == AF_INET6 && len >= (socklen_t)sizeof(struct sockaddr_in6) &&
               cfg.map_loopback &&
               IN6_IS_ADDR_LOOPBACK(&((const struct sockaddr_in6 *)sa)->sin6_addr)) {
        /* ::1 maps to the local IPv4 address, as 127.0.0.1 does. */
        memset(&v4, 0, sizeof v4);
        v4.sin_family = AF_INET;
        v4.sin_port = ((const struct sockaddr_in6 *)sa)->sin6_port;
        v4.sin_addr = cfg.local_v4;
        sa = (const struct sockaddr *)&v4;
        len = sizeof v4;
    }
    if (sa->sa_family == AF_INET) {
        if (len < (socklen_t)sizeof(struct sockaddr_in)) {
            errno = EINVAL;
            return -1;
        }
        const struct sockaddr_in *s = (const struct sockaddr_in *)sa;
        struct in_addr addr = s->sin_addr;
        if (cfg.map_loopback && (ntohl(addr.s_addr) >> 24) == 127) {
            addr = cfg.local_v4;
        }
        if (!inet_ntop(AF_INET, &addr, ip, sizeof ip)) return -1;
        port = ntohs(s->sin_port);
        family = 4;
    } else if (sa->sa_family == AF_INET6) {
        if (len < (socklen_t)sizeof(struct sockaddr_in6)) {
            errno = EINVAL;
            return -1;
        }
        const struct sockaddr_in6 *s = (const struct sockaddr_in6 *)sa;
        format_v6(s->sin6_addr.s6_addr, ip, sizeof ip);
        port = ntohs(s->sin6_port);
        family = 6;
    } else {
        errno = EAFNOSUPPORT;
        return -1;
    }
    memset(un, 0, sizeof *un);
    un->sun_family = AF_UNIX;
    // Build in a buffer the size of sun_path, then copy the bytes without the
    // NUL: an abstract name may fill sun_path[1..] to the last byte.
    char name[sizeof(un->sun_path)];
    int n = snprintf(name, sizeof name, "%s/%d/%s/%u",
                     cfg.prefix, family, ip, port);
    if (n <= 0 || (size_t)n >= sizeof name) {
        errno = ENAMETOOLONG;
        return -1;
    }
    memcpy(un->sun_path + 1, name, (size_t)n);
    *un_len = (socklen_t)(offsetof(struct sockaddr_un, sun_path) + 1 + (size_t)n);
    return 0;
}

/* Parse \0podnet-vnet/4/10.66.0.2/8080 back into a sockaddr. */
static int from_abstract_name(const char *name, struct sockaddr_storage *out, socklen_t *out_len) {
    char prefix[96];
    snprintf(prefix, sizeof prefix, "%s/", cfg.prefix);
    size_t plen = strlen(prefix);
    if (strncmp(name, prefix, plen) != 0) return -1;
    const char *rest = name + plen;
    const char *slash = strchr(rest, '/');
    if (!slash) return -1;
    int family = atoi(rest);
    char host[64];
    size_t hlen = (size_t)(slash - rest - 1) + 1; /* digits before '/' */
    if (hlen > 8) return -1;
    /* host is between slash+1 and the next slash */
    const char *host_start = slash + 1;
    const char *port_slash = strchr(host_start, '/');
    if (!port_slash) return -1;
    size_t host_len = (size_t)(port_slash - host_start);
    if (host_len >= sizeof host) return -1;
    memcpy(host, host_start, host_len);
    host[host_len] = '\0';
    unsigned port = (unsigned)atoi(port_slash + 1);
    memset(out, 0, sizeof *out);
    if (family == 4) {
        struct sockaddr_in *s = (struct sockaddr_in *)out;
        s->sin_family = AF_INET;
        s->sin_port = htons((uint16_t)port);
        if (inet_pton(AF_INET, host, &s->sin_addr) != 1) return -1;
        *out_len = sizeof *s;
        return 0;
    }
    if (family == 6) {
        struct sockaddr_in6 *s = (struct sockaddr_in6 *)out;
        s->sin6_family = AF_INET6;
        s->sin6_port = htons((uint16_t)port);
        if (inet_pton(AF_INET6, host, &s->sin6_addr) != 1) return -1;
        *out_len = sizeof *s;
        return 0;
    }
    return -1;
}

/* Recover a logical peer from an AF_UNIX abstract address, if it is ours. */
static int peer_from_unix(const struct sockaddr *sa, socklen_t len,
                          struct sockaddr_storage *out, socklen_t *out_len) {
    if (!sa || sa->sa_family != AF_UNIX) return -1;
    const struct sockaddr_un *un = (const struct sockaddr_un *)sa;
    socklen_t path_len = (socklen_t)(len - offsetof(struct sockaddr_un, sun_path));
    if (path_len <= 1 || un->sun_path[0] != '\0') {
        /* A filesystem path or an unnamed socket: not ours. */
        return -1;
    }
    /* An abstract name has no NUL at its end: the length is the end. Parse
       a copy with a NUL. Before, the parse read the bytes after the name
       (a peer of port 5555 read as 55552 with a '2' after it). */
    size_t name_len = (size_t)path_len - 1;
    if (name_len > sizeof un->sun_path - 1) name_len = sizeof un->sun_path - 1;
    char name[sizeof un->sun_path];
    memcpy(name, un->sun_path + 1, name_len);
    name[name_len] = '\0';
    return from_abstract_name(name, out, out_len);
}

/* ── control protocol (optional, switch mode) ─────────────────────────────── */

static int ctl_fd = -1;

static int ctl_connect(void) {
    if (ctl_fd >= 0) return ctl_fd;
    if (!cfg.control[0]) return -1;
    int fd = r_socket(AF_UNIX, SOCK_DGRAM, 0);
    if (fd < 0) return -1;
    struct sockaddr_un un;
    memset(&un, 0, sizeof un);
    un.sun_family = AF_UNIX;
    size_t clen = strlen(cfg.control);
    if (clen > sizeof un.sun_path - 2) clen = sizeof un.sun_path - 2;
    memcpy(un.sun_path + 1, cfg.control, clen);
    socklen_t len = (socklen_t)(offsetof(struct sockaddr_un, sun_path) + 1 + clen);
    if (r_connect(fd, (struct sockaddr *)&un, len) < 0) {
        r_close(fd);
        return -1;
    }
    ctl_fd = fd;
    return fd;
}

static void ctl_register(int op, int id, const struct sockaddr *sa) {
    if (!cfg.control[0]) return;
    int fd = ctl_connect();
    if (fd < 0) return;
    unsigned char payload[64];
    size_t off = 0;
    struct sockaddr_in v4;
    if (unmap_v4(sa, sizeof(struct sockaddr_in6), &v4)) sa = (const struct sockaddr *)&v4;
    payload[off++] = (unsigned char)(id >> 24);
    payload[off++] = (unsigned char)(id >> 16);
    payload[off++] = (unsigned char)(id >> 8);
    payload[off++] = (unsigned char)id;
    if (sa->sa_family == AF_INET) {
        const struct sockaddr_in *s = (const struct sockaddr_in *)sa;
        payload[off++] = 4;
        memcpy(payload + off, &s->sin_addr, 4);
        off += 4;
        payload[off++] = (unsigned char)(s->sin_port >> 8);
        payload[off++] = (unsigned char)(s->sin_port & 0xff);
    } else if (sa->sa_family == AF_INET6) {
        const struct sockaddr_in6 *s = (const struct sockaddr_in6 *)sa;
        payload[off++] = 6;
        memcpy(payload + off, &s->sin6_addr, 16);
        off += 16;
        payload[off++] = (unsigned char)(s->sin6_port >> 8);
        payload[off++] = (unsigned char)(s->sin6_port & 0xff);
    } else {
        return;
    }
    unsigned char frame[3 + 64];
    frame[0] = (unsigned char)op;
    frame[1] = (unsigned char)(off >> 8);
    frame[2] = (unsigned char)(off & 0xff);
    memcpy(frame + 3, payload, off);
    ssize_t n = send(fd, frame, off + 3, MSG_DONTWAIT);
    if (n < 0) {
        r_close(fd);
        ctl_fd = -1;
        logf_("control registration failed: %s\n", strerror(errno));
    }
}

/* ── netlink: the interfaces of the virtual network (T-392) ───────────────── */

/* A program that enumerates its interfaces asks the kernel by NETLINK_ROUTE
   (RTM_GETLINK, RTM_GETADDR), or calls getifaddrs or if_nameindex. Under the
   shim the program has one address, so the shim gives the same view to each
   of these: the interface lo (127.0.0.1/8 and ::1/128) and the interface
   vnet0 (the local address, /24). The host's interfaces do not show.

   socket(AF_NETLINK, *, NETLINK_ROUTE) is a socketpair(AF_UNIX, SOCK_DGRAM).
   The program has one end. The shim keeps the other end, the "kernel end".
   A request that the program sends with send, sendto or sendmsg goes to the
   kernel end, and the shim answers it at once into the kernel end, so that
   the answer waits on the end of the program. A request that the program
   writes with write(2) gets its answer at the next recv, recvfrom or
   recvmsg. Each answer carries the port id of the socket (nlmsg_pid), the
   one that getsockname gives: a program that compares the two (libtorrent
   does) drops each other answer.

   PODNET_VNET_HOST_NETLINK=1 turns this off: the host's kernel answers. */


#define NL_LO 1      /* the index of lo */
#define NL_VNET 2    /* the index of vnet0 */
#define NL_OPER_UNKNOWN 0
#define NL_OPER_UP 6
#ifndef IFF_LOWER_UP
#define IFF_LOWER_UP 0x10000 /* linux/if.h; net/if.h of glibc leaves it out */
#endif

static int nl_host(void) {
    return cfg.host_netlink;
}

static int fd_netlink_peer(int fd, uint32_t *pid) {
    int peer = -1;
    pthread_mutex_lock(&fd_lock);
    if (fd >= 0 && (size_t)fd < fd_table_len && fd_table[fd].is_netlink) {
        peer = fd_table[fd].nl_peer;
        if (pid) *pid = fd_table[fd].nl_pid;
    }
    pthread_mutex_unlock(&fd_lock);
    return peer;
}

/* The port id of a socket that is not bound: getpid() for the first one, as
   the kernel gives, then a negative number for each next one. */
static uint32_t nl_next_pid(void) {
    static int32_t next;
    static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
    pthread_mutex_lock(&lock);
    int32_t pid = next == 0 ? (int32_t)getpid() : next;
    next = next == 0 ? -4096 : next - 1;
    pthread_mutex_unlock(&lock);
    return (uint32_t)pid;
}

static int nl_socket(int type) {
    int sv[2];
    int flags = type & (SOCK_NONBLOCK | SOCK_CLOEXEC);
    if (r_socketpair(AF_UNIX, SOCK_DGRAM | flags, 0, sv) < 0) return -1;
    /* The kernel end never blocks the program and stays out of a child of exec. */
    int fl = r_fcntl(sv[1], F_GETFL);
    if (fl >= 0) r_fcntl(sv[1], F_SETFL, fl | O_NONBLOCK);
    r_fcntl(sv[1], F_SETFD, FD_CLOEXEC);
    pthread_mutex_lock(&fd_lock);
    if (table_ensure(sv[0]) < 0) {
        pthread_mutex_unlock(&fd_lock);
        r_close(sv[0]);
        r_close(sv[1]);
        errno = EMFILE;
        return -1;
    }
    memset(&fd_table[sv[0]], 0, sizeof fd_table[sv[0]]);
    fd_table[sv[0]].is_netlink = 1;
    fd_table[sv[0]].nl_peer = sv[1];
    fd_table[sv[0]].nl_pid = nl_next_pid();
    pthread_mutex_unlock(&fd_lock);
    logf_("socket(AF_NETLINK, NETLINK_ROUTE) -> fd=%d (answered in the process)\n", sv[0]);
    return sv[0];
}

/* Close the kernel end when no other descriptor of the program holds it. */
static void nl_release(int peer) {
    int used = 0;
    pthread_mutex_lock(&fd_lock);
    for (size_t i = 0; i < fd_table_len && !used; i++) {
        used = fd_table[i].is_netlink && fd_table[i].nl_peer == peer;
    }
    pthread_mutex_unlock(&fd_lock);
    if (!used) r_close(peer);
}

/* An answer under construction: whole messages, in one datagram. */
struct nl_out {
    unsigned char buf[4096];
    size_t len;
};

static struct nlmsghdr *nl_begin(struct nl_out *o, uint16_t type, uint16_t flags,
                                 uint32_t seq, uint32_t pid, const void *body, size_t body_len) {
    size_t need = NLMSG_SPACE(body_len);
    if (o->len + need > sizeof o->buf) return NULL;
    struct nlmsghdr *h = (struct nlmsghdr *)(o->buf + o->len);
    memset(h, 0, need);
    h->nlmsg_len = (uint32_t)NLMSG_LENGTH(body_len);
    h->nlmsg_type = type;
    h->nlmsg_flags = flags;
    h->nlmsg_seq = seq;
    h->nlmsg_pid = pid;
    memcpy(NLMSG_DATA(h), body, body_len);
    o->len += need;
    return h;
}

static void nl_attr(struct nl_out *o, struct nlmsghdr *h, uint16_t type, const void *data, size_t len) {
    size_t need = RTA_SPACE(len);
    if (!h || o->len + need > sizeof o->buf) return;
    struct rtattr *a = (struct rtattr *)(o->buf + o->len);
    memset(a, 0, need);
    a->rta_type = type;
    a->rta_len = (unsigned short)RTA_LENGTH(len);
    memcpy(RTA_DATA(a), data, len);
    o->len += need;
    h->nlmsg_len = (uint32_t)(NLMSG_ALIGN(h->nlmsg_len) + need);
}

static void nl_attr_u32(struct nl_out *o, struct nlmsghdr *h, uint16_t type, uint32_t v) {
    nl_attr(o, h, type, &v, sizeof v);
}

/* The two links. */
static void nl_link(struct nl_out *o, int index, uint16_t flags, uint32_t seq, uint32_t pid) {
    struct ifinfomsg ifi;
    memset(&ifi, 0, sizeof ifi);
    ifi.ifi_family = AF_UNSPEC;
    ifi.ifi_index = index;
    unsigned char mac[6] = {0, 0, 0, 0, 0, 0};
    unsigned char bcast[6] = {0, 0, 0, 0, 0, 0};
    const char *name = "lo";
    uint32_t mtu = 65536;
    unsigned char oper = NL_OPER_UNKNOWN;
    if (index == NL_LO) {
        ifi.ifi_type = ARPHRD_LOOPBACK;
        ifi.ifi_flags = IFF_UP | IFF_LOOPBACK | IFF_RUNNING | IFF_LOWER_UP;
    } else {
        ifi.ifi_type = ARPHRD_ETHER;
        ifi.ifi_flags = IFF_UP | IFF_BROADCAST | IFF_RUNNING | IFF_MULTICAST | IFF_LOWER_UP;
        /* A locally administered address that holds the local address. */
        const unsigned char *a = (const unsigned char *)&cfg.local_v4.s_addr;
        mac[0] = 0x02;
        mac[1] = 0x00;
        memcpy(mac + 2, a, 4);
        memset(bcast, 0xff, sizeof bcast);
        name = "vnet0";
        mtu = 1500;
        oper = NL_OPER_UP;
    }
    struct nlmsghdr *h = nl_begin(o, RTM_NEWLINK, flags, seq, pid, &ifi, sizeof ifi);
    nl_attr(o, h, IFLA_IFNAME, name, strlen(name) + 1);
    nl_attr_u32(o, h, IFLA_MTU, mtu);
    nl_attr_u32(o, h, IFLA_TXQLEN, 1000);
    nl_attr(o, h, IFLA_OPERSTATE, &oper, 1);
    nl_attr(o, h, IFLA_ADDRESS, mac, sizeof mac);
    nl_attr(o, h, IFLA_BROADCAST, bcast, sizeof bcast);
}

/* The three addresses, of FAMILY (AF_UNSPEC: each). */
static void nl_addrs(struct nl_out *o, int family, uint32_t seq, uint32_t pid) {
    struct ifaddrmsg ifa;
    struct nlmsghdr *h;
    if (family == AF_UNSPEC || family == AF_INET) {
        struct in_addr lo4;
        lo4.s_addr = htonl(INADDR_LOOPBACK);
        memset(&ifa, 0, sizeof ifa);
        ifa.ifa_family = AF_INET;
        ifa.ifa_prefixlen = 8;
        ifa.ifa_flags = IFA_F_PERMANENT;
        ifa.ifa_scope = RT_SCOPE_HOST;
        ifa.ifa_index = NL_LO;
        h = nl_begin(o, RTM_NEWADDR, NLM_F_MULTI, seq, pid, &ifa, sizeof ifa);
        nl_attr(o, h, IFA_ADDRESS, &lo4, sizeof lo4);
        nl_attr(o, h, IFA_LOCAL, &lo4, sizeof lo4);
        nl_attr(o, h, IFA_LABEL, "lo", 3);

        struct in_addr local = cfg.local_v4;
        struct in_addr bcast;
        bcast.s_addr = local.s_addr | htonl(0xffu);
        memset(&ifa, 0, sizeof ifa);
        ifa.ifa_family = AF_INET;
        ifa.ifa_prefixlen = 24;
        ifa.ifa_flags = IFA_F_PERMANENT;
        ifa.ifa_scope = RT_SCOPE_UNIVERSE;
        ifa.ifa_index = NL_VNET;
        h = nl_begin(o, RTM_NEWADDR, NLM_F_MULTI, seq, pid, &ifa, sizeof ifa);
        nl_attr(o, h, IFA_ADDRESS, &local, sizeof local);
        nl_attr(o, h, IFA_LOCAL, &local, sizeof local);
        nl_attr(o, h, IFA_BROADCAST, &bcast, sizeof bcast);
        nl_attr(o, h, IFA_LABEL, "vnet0", 6);
    }
    if (family == AF_UNSPEC || family == AF_INET6) {
        struct in6_addr lo6 = IN6ADDR_LOOPBACK_INIT;
        memset(&ifa, 0, sizeof ifa);
        ifa.ifa_family = AF_INET6;
        ifa.ifa_prefixlen = 128;
        ifa.ifa_flags = IFA_F_PERMANENT;
        ifa.ifa_scope = RT_SCOPE_HOST;
        ifa.ifa_index = NL_LO;
        h = nl_begin(o, RTM_NEWADDR, NLM_F_MULTI, seq, pid, &ifa, sizeof ifa);
        nl_attr(o, h, IFA_ADDRESS, &lo6, sizeof lo6);
    }
}

static void nl_send_out(int peer, struct nl_out *o) {
    if (o->len == 0) return;
    if (r_send(peer, o->buf, o->len, MSG_DONTWAIT | MSG_NOSIGNAL) < 0) {
        logf_("netlink: the answer of %zu bytes was not queued: %s\n", o->len, strerror(errno));
    }
    o->len = 0;
}

static void nl_error(struct nl_out *o, const struct nlmsghdr *req, int error, uint32_t pid) {
    struct nlmsgerr e;
    memset(&e, 0, sizeof e);
    e.error = error;
    e.msg = *req;
    nl_begin(o, NLMSG_ERROR, 0, req->nlmsg_seq, pid, &e, sizeof e);
}

static void nl_done(struct nl_out *o, const struct nlmsghdr *req, uint32_t pid) {
    int32_t zero = 0;
    nl_begin(o, NLMSG_DONE, NLM_F_MULTI, req->nlmsg_seq, pid, &zero, sizeof zero);
}

/* Answer one request. */
static void nl_answer(int peer, const struct nlmsghdr *req, uint32_t pid) {
    struct nl_out o;
    o.len = 0;
    size_t body = req->nlmsg_len >= NLMSG_HDRLEN ? req->nlmsg_len - NLMSG_HDRLEN : 0;
    int family = body >= 1 ? ((const unsigned char *)NLMSG_DATA(req))[0] : AF_UNSPEC;
    int dump = (req->nlmsg_flags & NLM_F_DUMP) == NLM_F_DUMP;
    int type = req->nlmsg_type;
    logf_("netlink: type=%d flags=0x%x family=%d\n", type, req->nlmsg_flags, family);
    if (type == RTM_GETLINK && dump) {
        if (family == AF_UNSPEC || family == AF_PACKET) {
            nl_link(&o, NL_LO, NLM_F_MULTI, req->nlmsg_seq, pid);
            nl_link(&o, NL_VNET, NLM_F_MULTI, req->nlmsg_seq, pid);
        }
        nl_send_out(peer, &o);
        nl_done(&o, req, pid);
    } else if (type == RTM_GETLINK) {
        int index = 0;
        if (body >= sizeof(struct ifinfomsg)) {
            const struct ifinfomsg *ifi = NLMSG_DATA(req);
            index = ifi->ifi_index;
            /* By name: IFLA_IFNAME. */
            const struct rtattr *a = (const struct rtattr *)((const char *)ifi + NLMSG_ALIGN(sizeof *ifi));
            int left = (int)(body - NLMSG_ALIGN(sizeof *ifi));
            for (; index == 0 && RTA_OK(a, left); a = RTA_NEXT(a, left)) {
                if (a->rta_type != IFLA_IFNAME) continue;
                const char *n = RTA_DATA(a);
                if (strncmp(n, "lo", RTA_PAYLOAD(a)) == 0) index = NL_LO;
                else if (strncmp(n, "vnet0", RTA_PAYLOAD(a)) == 0) index = NL_VNET;
                else index = -1;
            }
        }
        if (index == NL_LO || index == NL_VNET) {
            nl_link(&o, index, 0, req->nlmsg_seq, pid);
            if (req->nlmsg_flags & NLM_F_ACK) {
                nl_send_out(peer, &o);
                nl_error(&o, req, 0, pid);
            }
        } else {
            nl_error(&o, req, -ENODEV, pid);
        }
    } else if (type == RTM_GETADDR && dump) {
        nl_addrs(&o, family, req->nlmsg_seq, pid);
        nl_send_out(peer, &o);
        nl_done(&o, req, pid);
    } else if (dump && (type & 3) == 2 && type >= RTM_BASE) {
        /* Each other dump (routes, neighbours, rules): empty. */
        nl_done(&o, req, pid);
    } else if (type >= RTM_BASE && (type & 3) == 2) {
        nl_error(&o, req, -EOPNOTSUPP, pid);
    } else if (type >= RTM_BASE) {
        /* A change (NEW, DEL, SET): the program may not change the network. */
        nl_error(&o, req, -EPERM, pid);
    } else if (req->nlmsg_flags & NLM_F_ACK) {
        nl_error(&o, req, 0, pid);
    }
    nl_send_out(peer, &o);
}

/* Answer each request that waits on the kernel end. */
static void nl_pump(int peer, uint32_t pid) {
    for (;;) {
        union {
            unsigned char buf[8192];
            struct nlmsghdr align;
        } in;
        int saved = errno;
        ssize_t n = r_recv(peer, in.buf, sizeof in.buf, MSG_DONTWAIT);
        errno = saved;
        if (n <= 0) return;
        int left = (int)n;
        for (struct nlmsghdr *h = &in.align; NLMSG_OK(h, left); h = NLMSG_NEXT(h, left)) {
            nl_answer(peer, h, pid);
        }
    }
}

/* The kernel's address: AF_NETLINK, port 0. */
static void nl_kernel_name(void *sa, socklen_t *len) {
    struct sockaddr_nl k;
    memset(&k, 0, sizeof k);
    k.nl_family = AF_NETLINK;
    if (!sa || !len) return;
    memcpy(sa, &k, *len < sizeof k ? *len : sizeof k);
    *len = sizeof k;
}

static int nl_bind(int fd, const struct sockaddr *sa, socklen_t len) {
    if (!sa || len < sizeof(struct sockaddr_nl) || sa->sa_family != AF_NETLINK) {
        errno = EINVAL;
        return -1;
    }
    const struct sockaddr_nl *nl = (const struct sockaddr_nl *)sa;
    if (nl->nl_pid != 0) {
        pthread_mutex_lock(&fd_lock);
        if ((size_t)fd < fd_table_len) fd_table[fd].nl_pid = nl->nl_pid;
        pthread_mutex_unlock(&fd_lock);
    }
    return 0;
}

static int nl_getsockname(int fd, struct sockaddr *sa, socklen_t *len) {
    uint32_t pid = 0;
    fd_netlink_peer(fd, &pid);
    struct sockaddr_storage ss;
    memset(&ss, 0, sizeof ss);
    struct sockaddr_nl *nl = (struct sockaddr_nl *)&ss;
    nl->nl_family = AF_NETLINK;
    nl->nl_pid = pid;
    return copy_sockaddr(sa, len, &ss, sizeof *nl);
}

static ssize_t nl_sendmsg(int fd, int peer, uint32_t pid, const struct msghdr *msg, int flags) {
    struct msghdr copy = *msg;
    copy.msg_name = NULL;
    copy.msg_namelen = 0;
    ssize_t rc = r_sendmsg(fd, &copy, flags | MSG_NOSIGNAL);
    if (rc >= 0) nl_pump(peer, pid);
    return rc;
}

static ssize_t nl_recvmsg(int fd, int peer, uint32_t pid, struct msghdr *msg, int flags) {
    nl_pump(peer, pid);
    void *name = msg->msg_name;
    socklen_t name_len = msg->msg_namelen;
    msg->msg_name = NULL;
    msg->msg_namelen = 0;
    ssize_t rc = r_recvmsg(fd, msg, flags);
    msg->msg_name = name;
    msg->msg_namelen = name_len;
    if (rc >= 0 && name) nl_kernel_name(name, &msg->msg_namelen);
    return rc;
}

/* getifaddrs, if_nameindex and their kin give the same two interfaces. The
   shim allocates each list in one block, and frees only its own blocks: the
   lists of the host's libc go to the host's free functions. */

static void *own_blocks[64];
static pthread_mutex_t own_lock = PTHREAD_MUTEX_INITIALIZER;

static void own_add(void *p) {
    pthread_mutex_lock(&own_lock);
    for (size_t i = 0; i < sizeof own_blocks / sizeof own_blocks[0]; i++) {
        if (!own_blocks[i]) {
            own_blocks[i] = p;
            break;
        }
    }
    pthread_mutex_unlock(&own_lock);
}

/* True when P is a block of the shim; the block leaves the list. */
static int own_take(void *p) {
    int found = 0;
    pthread_mutex_lock(&own_lock);
    for (size_t i = 0; i < sizeof own_blocks / sizeof own_blocks[0] && !found; i++) {
        if (p && own_blocks[i] == p) {
            own_blocks[i] = NULL;
            found = 1;
        }
    }
    pthread_mutex_unlock(&own_lock);
    return found;
}

struct own_ifa {
    struct ifaddrs ifa;
    struct sockaddr_storage addr, mask, extra;
    char name[IF_NAMESIZE];
};

static void own_set4(struct sockaddr_storage *ss, uint32_t host_order) {
    struct sockaddr_in *a = (struct sockaddr_in *)ss;
    a->sin_family = AF_INET;
    a->sin_addr.s_addr = htonl(host_order);
}

int getifaddrs(struct ifaddrs **out) {
    ensure_init();
    if (nl_host()) return r_getifaddrs(out);
    if (!out) {
        errno = EINVAL;
        return -1;
    }
    struct own_ifa *n = calloc(3, sizeof *n);
    if (!n) return -1;
    uint32_t local = ntohl(cfg.local_v4.s_addr);
    /* lo, 127.0.0.1/8 */
    snprintf(n[0].name, IF_NAMESIZE, "lo");
    n[0].ifa.ifa_flags = IFF_UP | IFF_LOOPBACK | IFF_RUNNING | IFF_LOWER_UP;
    own_set4(&n[0].addr, INADDR_LOOPBACK);
    own_set4(&n[0].mask, 0xff000000u);
    /* vnet0, the local address /24 */
    snprintf(n[1].name, IF_NAMESIZE, "vnet0");
    n[1].ifa.ifa_flags = IFF_UP | IFF_BROADCAST | IFF_RUNNING | IFF_MULTICAST | IFF_LOWER_UP;
    own_set4(&n[1].addr, local);
    own_set4(&n[1].mask, 0xffffff00u);
    own_set4(&n[1].extra, local | 0xffu);
    n[1].ifa.ifa_broadaddr = (struct sockaddr *)&n[1].extra;
    /* lo, ::1/128 */
    snprintf(n[2].name, IF_NAMESIZE, "lo");
    n[2].ifa.ifa_flags = n[0].ifa.ifa_flags;
    struct sockaddr_in6 *a6 = (struct sockaddr_in6 *)&n[2].addr;
    a6->sin6_family = AF_INET6;
    a6->sin6_addr = in6addr_loopback;
    struct sockaddr_in6 *m6 = (struct sockaddr_in6 *)&n[2].mask;
    m6->sin6_family = AF_INET6;
    memset(&m6->sin6_addr, 0xff, sizeof m6->sin6_addr);
    for (int i = 0; i < 3; i++) {
        n[i].ifa.ifa_name = n[i].name;
        n[i].ifa.ifa_addr = (struct sockaddr *)&n[i].addr;
        n[i].ifa.ifa_netmask = (struct sockaddr *)&n[i].mask;
        n[i].ifa.ifa_next = i < 2 ? &n[i + 1].ifa : NULL;
    }
    own_add(n);
    *out = &n[0].ifa;
    return 0;
}

void freeifaddrs(struct ifaddrs *ifa) {
    ensure_init();
    if (own_take(ifa)) {
        free(ifa);
        return;
    }
    if (r_freeifaddrs) r_freeifaddrs(ifa);
}

unsigned int if_nametoindex(const char *name) {
    ensure_init();
    if (nl_host()) return r_if_nametoindex(name);
    if (name && strcmp(name, "lo") == 0) return NL_LO;
    if (name && strcmp(name, "vnet0") == 0) return NL_VNET;
    errno = ENODEV;
    return 0;
}

char *if_indextoname(unsigned int index, char name[IF_NAMESIZE]) {
    ensure_init();
    if (nl_host()) return r_if_indextoname(index, name);
    if (index != NL_LO && index != NL_VNET) {
        errno = ENXIO;
        return NULL;
    }
    snprintf(name, IF_NAMESIZE, "%s", index == NL_LO ? "lo" : "vnet0");
    return name;
}

struct own_index {
    struct if_nameindex list[3];
    char names[2][IF_NAMESIZE];
};

struct if_nameindex *if_nameindex(void) {
    ensure_init();
    if (nl_host()) return r_if_nameindex();
    struct own_index *n = calloc(1, sizeof *n);
    if (!n) return NULL;
    snprintf(n->names[0], IF_NAMESIZE, "lo");
    snprintf(n->names[1], IF_NAMESIZE, "vnet0");
    n->list[0].if_index = NL_LO;
    n->list[0].if_name = n->names[0];
    n->list[1].if_index = NL_VNET;
    n->list[1].if_name = n->names[1];
    own_add(n);
    return n->list;
}

void if_freenameindex(struct if_nameindex *list) {
    ensure_init();
    if (own_take(list)) {
        free(list);
        return;
    }
    if (r_if_freenameindex) r_if_freenameindex(list);
}

/* ── interposed API ───────────────────────────────────────────────────────── */

int socket(int domain, int type, int protocol) {
    ensure_init();
    if (domain == AF_NETLINK && protocol == NETLINK_ROUTE && !nl_host()) return nl_socket(type);
    if (domain == AF_INET || domain == AF_INET6) {
        if (type == SOCK_RAW) {
            errno = EAFNOSUPPORT;
            return -1;
        }
        int fd = r_socket(AF_UNIX, type, 0);
        if (fd >= 0) {
            fd_mark(fd, domain, type);
            /* Each datagram carries the user of its sender. */
            if ((type & 0xf) == SOCK_DGRAM) {
                int one = 1;
                r_setsockopt(fd, SOL_SOCKET, SO_PASSCRED, &one, sizeof one);
            }
            logf_("socket(%s) -> AF_UNIX fd=%d\n",
                  domain == AF_INET ? "AF_INET" : "AF_INET6", fd);
        }
        return fd;
    }
    return r_socket(domain, type, protocol);
}

int socketpair(int domain, int type, int protocol, int sv[2]) {
    ensure_init();
    if (domain == AF_INET || domain == AF_INET6) {
        errno = EOPNOTSUPP;
        return -1;
    }
    return r_socketpair(domain, type, protocol, sv);
}

/* The port of an AF_INET or AF_INET6 address, and a copy with another one. */
static int sa_port(const struct sockaddr *sa, socklen_t len) {
    if (sa->sa_family == AF_INET && len >= (socklen_t)sizeof(struct sockaddr_in))
        return ntohs(((const struct sockaddr_in *)sa)->sin_port);
    if (sa->sa_family == AF_INET6 && len >= (socklen_t)sizeof(struct sockaddr_in6))
        return ntohs(((const struct sockaddr_in6 *)sa)->sin6_port);
    return -1;
}

static void sa_set_port(struct sockaddr_storage *ss, int port) {
    if (ss->ss_family == AF_INET) ((struct sockaddr_in *)ss)->sin_port = htons(port);
    else if (ss->ss_family == AF_INET6) ((struct sockaddr_in6 *)ss)->sin6_port = htons(port);
}

/* A bind to the wildcard address (INADDR_ANY or in6addr_any) means each
   address of this host. Under the shim the host has one address, the local
   address: the bind takes it, so that a peer that dials the local address
   finds the name (T-391). An AF_INET6 descriptor gets the mapped form, as
   autobind gives it. Return 1 and fill ss when sa is a wildcard. */
static int wildcard_to_local(const struct sockaddr *sa, socklen_t len,
                             struct sockaddr_storage *ss, socklen_t *out) {
    if (!sa) return 0;
    memset(ss, 0, sizeof *ss);
    if (sa->sa_family == AF_INET && len >= (socklen_t)sizeof(struct sockaddr_in)) {
        const struct sockaddr_in *s = (const struct sockaddr_in *)sa;
        if (s->sin_addr.s_addr != htonl(INADDR_ANY)) return 0;
        struct sockaddr_in *d = (struct sockaddr_in *)ss;
        *d = *s;
        d->sin_addr = cfg.local_v4;
        *out = sizeof *d;
        return 1;
    }
    if (sa->sa_family == AF_INET6 && len >= (socklen_t)sizeof(struct sockaddr_in6)) {
        const struct sockaddr_in6 *s = (const struct sockaddr_in6 *)sa;
        if (!IN6_IS_ADDR_UNSPECIFIED(&s->sin6_addr)) return 0;
        struct sockaddr_in6 *d = (struct sockaddr_in6 *)ss;
        *d = *s;
        memset(d->sin6_addr.s6_addr, 0, 16);
        d->sin6_addr.s6_addr[10] = 0xff;
        d->sin6_addr.s6_addr[11] = 0xff;
        memcpy(d->sin6_addr.s6_addr + 12, &cfg.local_v4, 4);
        *out = sizeof *d;
        return 1;
    }
    return 0;
}

static int bind_one(int fd, const struct sockaddr *sa, socklen_t len) {
    struct sockaddr_un un;
    socklen_t un_len;
    struct sockaddr_storage local;
    socklen_t local_len;
    if (wildcard_to_local(sa, len, &local, &local_len)) {
        sa = (const struct sockaddr *)&local;
        len = local_len;
    }
    if (to_abstract(sa, len, &un, &un_len) < 0) return -1;
    int rc = r_bind(fd, (struct sockaddr *)&un, un_len);
    if (rc == 0) {
        fd_record_local(fd, sa, len);
        logf_("bind -> %s\n", un.sun_path + 1);
        ctl_register(1, fd, sa);
    }
    return rc;
}

int bind(int fd, const struct sockaddr *sa, socklen_t len) {
    ensure_init();
    if (fd_netlink_peer(fd, NULL) >= 0) return nl_bind(fd, sa, len);
    if (!fd_is_virtual(fd)) return r_bind(fd, sa, len);
    if (sa == NULL || sa_port(sa, len) != 0) return bind_one(fd, sa, len);
    /* Port 0: the kernel would pick a free port. Pick one from the
       ephemeral range (49152-65535), from a random start, and skip the
       ones in use. Before, port 0 was a name of its own, and a second bind
       to port 0 got EADDRINUSE. */
    struct sockaddr_storage ss;
    if (len > (socklen_t)sizeof ss) len = sizeof ss;
    memcpy(&ss, sa, len);
    unsigned seed = (unsigned)getpid() ^ (unsigned)time(NULL) ^ (unsigned)fd * 2654435761u;
    int start = 49152 + (int)(rand_r(&seed) % 16384);
    for (int i = 0; i < 16384; i++) {
        int port = 49152 + (start - 49152 + i) % 16384;
        sa_set_port(&ss, port);
        int saved = errno;
        if (bind_one(fd, (struct sockaddr *)&ss, len) == 0) return 0;
        if (errno != EADDRINUSE) return -1;
        errno = saved;
    }
    errno = EADDRINUSE;
    return -1;
}

/* Issue 193: a stream socket with no name gets one at connect, as the
   kernel does: the local address of the shim and a port of the ephemeral
   range. getsockname then gives it, and the server sees it as the peer.
   It is not a listener: no registration with the control socket. */
static void autobind(int fd) {
    pthread_mutex_lock(&fd_lock);
    int skip = fd < 0 || (size_t)fd >= fd_table_len || fd_table[fd].local_len != 0 ||
               fd_table[fd].is_dgram;
    int family = skip ? 0 : fd_table[fd].family;
    pthread_mutex_unlock(&fd_lock);
    if (skip) return;
    struct sockaddr_storage ss;
    socklen_t len;
    memset(&ss, 0, sizeof ss);
    if (family == AF_INET6) {
        struct sockaddr_in6 *s6 = (struct sockaddr_in6 *)&ss;
        s6->sin6_family = AF_INET6;
        s6->sin6_addr.s6_addr[10] = 0xff;
        s6->sin6_addr.s6_addr[11] = 0xff;
        memcpy(s6->sin6_addr.s6_addr + 12, &cfg.local_v4, 4);
        len = sizeof *s6;
    } else {
        struct sockaddr_in *s4 = (struct sockaddr_in *)&ss;
        s4->sin_family = AF_INET;
        s4->sin_addr = cfg.local_v4;
        len = sizeof *s4;
    }
    int saved = errno;
    unsigned seed = (unsigned)getpid() ^ (unsigned)time(NULL) ^ (unsigned)fd * 2246822519u;
    int start = 49152 + (int)(rand_r(&seed) % 16384);
    for (int i = 0; i < 16384; i++) {
        int port = 49152 + (start - 49152 + i) % 16384;
        sa_set_port(&ss, port);
        struct sockaddr_un un;
        socklen_t un_len;
        if (to_abstract((struct sockaddr *)&ss, len, &un, &un_len) < 0) break;
        if (r_bind(fd, (struct sockaddr *)&un, un_len) == 0) {
            fd_record_local(fd, (struct sockaddr *)&ss, len);
            logf_("autobind -> %s\n", un.sun_path + 1);
            break;
        }
        if (errno != EADDRINUSE) break;
    }
    /* A failed autobind leaves the socket with no name, as before. */
    errno = saved;
}

/* Issue 308: an abstract name has no file permissions, so each local user
   can bind or connect to it. A stream socket of the shim takes the same
   user only: the user of the peer (SO_PEERCRED) must be the effective
   user. */
static int peer_is_own(int fd) {
    struct ucred cred;
    socklen_t len = sizeof cred;
    if (r_getsockopt(fd, SOL_SOCKET, SO_PEERCRED, &cred, &len) != 0) return 0;
    return cred.uid == geteuid();
}

/* Put a new socket of the same type and flags in the place of fd, so that
   the connection to a listener of another user is gone and fd is a socket
   with no peer again. 0 on success. */
static int fresh_socket(int fd, int type) {
    int fl = r_fcntl(fd, F_GETFL);
    int fdfl = r_fcntl(fd, F_GETFD);
    int nfd = r_socket(AF_UNIX, type, 0);
    if (nfd < 0) return -1;
    if (fl >= 0) r_fcntl(nfd, F_SETFL, fl);
    int rc = r_dup2(nfd, fd);
    r_close(nfd);
    if (rc < 0) return -1;
    if (fdfl >= 0) r_fcntl(fd, F_SETFD, fdfl);
    return 0;
}

/* ── a stream connect through the SOCKS5 front door ───────────────── */

static int socks_wait(int fd, short events) {
    struct pollfd p = {.fd = fd, .events = events};
    for (;;) {
        int r = poll(&p, 1, -1);
        if (r < 0 && errno == EINTR) continue;
        return r < 0 ? -1 : 0;
    }
}

static int socks_io(int fd, unsigned char *buf, size_t n, int out) {
    size_t done = 0;
    while (done < n) {
        ssize_t r = out ? send(fd, buf + done, n - done, MSG_NOSIGNAL)
                        : recv(fd, buf + done, n - done, 0);
        if (r == 0 && !out) {
            errno = ECONNRESET;
            return -1;
        }
        if (r < 0) {
            if (errno == EINTR) continue;
            if (errno == EAGAIN || errno == EWOULDBLOCK) {
                if (socks_wait(fd, out ? POLLOUT : POLLIN) < 0) return -1;
                continue;
            }
            return -1;
        }
        done += (size_t)r;
    }
    return 0;
}

/* The SOCKS5 greeting and CONNECT for sa on fd, with no authentication. */
static int socks5_connect(int fd, const struct sockaddr *sa) {
    unsigned char buf[262] = {5, 1, 0};
    if (socks_io(fd, buf, 3, 1) < 0 || socks_io(fd, buf, 2, 0) < 0) return -1;
    if (buf[0] != 5 || buf[1] != 0) {
        errno = EACCES;
        return -1;
    }
    size_t n = 4;
    buf[0] = 5;
    buf[1] = 1;
    buf[2] = 0;
    if (sa->sa_family == AF_INET) {
        const struct sockaddr_in *s = (const struct sockaddr_in *)sa;
        buf[3] = 1;
        memcpy(buf + n, &s->sin_addr, 4);
        n += 4;
        memcpy(buf + n, &s->sin_port, 2);
    } else {
        const struct sockaddr_in6 *s = (const struct sockaddr_in6 *)sa;
        buf[3] = 4;
        memcpy(buf + n, &s->sin6_addr, 16);
        n += 16;
        memcpy(buf + n, &s->sin6_port, 2);
    }
    n += 2;
    if (socks_io(fd, buf, n, 1) < 0 || socks_io(fd, buf, 4, 0) < 0) return -1;
    if (buf[0] != 5) {
        errno = EPROTO;
        return -1;
    }
    if (buf[1] != 0) {
        errno = buf[1] == 5 ? ECONNREFUSED : EHOSTUNREACH;
        return -1;
    }
    size_t skip = buf[3] == 1 ? 4 : buf[3] == 4 ? 16 : 0;
    if (buf[3] == 3) {
        if (socks_io(fd, buf, 1, 0) < 0) return -1;
        skip = buf[0];
    }
    return socks_io(fd, buf, skip + 2, 0);
}

/* Connect the virtual stream socket fd to the front door and ask it for sa.
   A blocking step: O_NONBLOCK is off until the answer of the door. */
static int connect_through_door(int fd, const struct sockaddr *sa, socklen_t len) {
    struct sockaddr_un un;
    memset(&un, 0, sizeof un);
    un.sun_family = AF_UNIX;
    snprintf(un.sun_path, sizeof un.sun_path, "%s", cfg.socks);
    socklen_t un_len = (socklen_t)(offsetof(struct sockaddr_un, sun_path) + strlen(un.sun_path) + 1);
    int flags = r_fcntl(fd, F_GETFL);
    if (flags >= 0 && (flags & O_NONBLOCK)) r_fcntl(fd, F_SETFL, flags & ~O_NONBLOCK);
    int rc = r_connect(fd, (struct sockaddr *)&un, un_len);
    if (rc == 0) rc = socks5_connect(fd, sa);
    int saved = errno;
    if (rc < 0) fresh_socket(fd, SOCK_STREAM);
    if (flags >= 0 && (flags & O_NONBLOCK)) r_fcntl(fd, F_SETFL, flags);
    if (rc < 0) {
        logf_("connect -> door %s failed: %s\n", cfg.socks, strerror(saved));
        errno = saved;
        return -1;
    }
    fd_record_peer(fd, sa, len);
    logf_("connect -> door %s\n", cfg.socks);
    return 0;
}

/* Connect the virtual stream socket fd to the unix socket path of a
   forward. The peer is the TCP address that the program asked for. A
   missing path gives ECONNREFUSED, as a TCP port with no listener does.
   The owner of the path decides with its file permissions: no same-user
   check. */
static int connect_forward(int fd, const struct sockaddr *sa, socklen_t len,
                           const struct forward *f) {
    struct sockaddr_un un;
    memset(&un, 0, sizeof un);
    un.sun_family = AF_UNIX;
    snprintf(un.sun_path, sizeof un.sun_path, "%s", f->path);
    socklen_t un_len = (socklen_t)(offsetof(struct sockaddr_un, sun_path) + strlen(un.sun_path) + 1);
    autobind(fd);
    /* A blocking step, as for the door: a full backlog waits, and does not
       give EAGAIN to a program that expects EINPROGRESS. */
    int flags = r_fcntl(fd, F_GETFL);
    if (flags >= 0 && (flags & O_NONBLOCK)) r_fcntl(fd, F_SETFL, flags & ~O_NONBLOCK);
    int rc = r_connect(fd, (struct sockaddr *)&un, un_len);
    int err = errno;
    if (flags >= 0 && (flags & O_NONBLOCK)) r_fcntl(fd, F_SETFL, flags);
    errno = err;
    if (rc < 0) {
        int saved = errno == ENOENT ? ECONNREFUSED : errno;
        logf_("connect -> forward %s failed: %s\n", f->path, strerror(errno));
        errno = saved;
        return -1;
    }
    fd_record_peer(fd, sa, len);
    logf_("connect -> forward %s\n", f->path);
    return 0;
}

int connect(int fd, const struct sockaddr *sa, socklen_t len) {
    ensure_init();
    if (fd_netlink_peer(fd, NULL) >= 0) return 0; /* the kernel is the one peer */
    if (!fd_is_virtual(fd)) return r_connect(fd, sa, len);
    const struct forward *f = forward_match(sa, len);
    if (f) {
        int type = 0;
        socklen_t type_len = sizeof type;
        if (r_getsockopt(fd, SOL_SOCKET, SO_TYPE, &type, &type_len) == 0 &&
            type == SOCK_STREAM) {
            return connect_forward(fd, sa, len, f);
        }
    }
    if (cfg.socks[0] && routes_len > 0 && sa &&
        (sa->sa_family == AF_INET || sa->sa_family == AF_INET6) && route_match(sa)) {
        int type = 0;
        socklen_t type_len = sizeof type;
        if (r_getsockopt(fd, SOL_SOCKET, SO_TYPE, &type, &type_len) == 0 &&
            type == SOCK_STREAM) {
            return connect_through_door(fd, sa, len);
        }
    }
    struct sockaddr_un un;
    socklen_t un_len;
    if (to_abstract(sa, len, &un, &un_len) < 0) return -1;
    autobind(fd);
    int rc = r_connect(fd, (struct sockaddr *)&un, un_len);
    int type = 0;
    socklen_t type_len = sizeof type;
    if (rc == 0 && r_getsockopt(fd, SOL_SOCKET, SO_TYPE, &type, &type_len) == 0 &&
        type == SOCK_STREAM && !peer_is_own(fd)) {
        logf_("connect -> %s refused: the listener is of another user\n", un.sun_path + 1);
        fresh_socket(fd, type);
        errno = ECONNREFUSED;
        return -1;
    }
    if (rc == 0) {
        fd_record_peer(fd, sa, len);
        logf_("connect -> %s\n", un.sun_path + 1);
    } else {
        logf_("connect -> %s failed: %s\n", un.sun_path + 1, strerror(errno));
    }
    return rc;
}

int listen(int fd, int backlog) {
    ensure_init();
    return r_listen(fd, backlog);
}

static socklen_t wildcard_addr(int family, struct sockaddr_storage *out);

static int finish_accept(int listener, int client, const struct sockaddr_storage *raw,
                         socklen_t raw_len, struct sockaddr *sa, socklen_t *len) {
    if (client < 0) return client;
    pthread_mutex_lock(&fd_lock);
    int family = 0;
    struct sockaddr_storage local;
    socklen_t local_len = 0;
    if ((size_t)listener < fd_table_len) {
        family = fd_table[listener].family;
        local = fd_table[listener].local;
        local_len = fd_table[listener].local_len;
    }
    pthread_mutex_unlock(&fd_lock);
    fd_mark(client, family ? family : AF_INET, 0);
    if (local_len) fd_record_local(client, (struct sockaddr *)&local, local_len);
    /* The logical peer: the podnet-vnet name of the client, from the whole
       answer of the kernel (a local buffer, so a 16-byte sockaddr_in of the
       caller does not cut the abstract name). A client that bound no name
       gets the wildcard address of the family, as a peer of port 0: a
       length of 0 is not an answer that accept(2) gives for TCP, and Perl's
       accept() reads it as a failure. */
    struct sockaddr_storage peer;
    socklen_t peer_len = 0;
    if (peer_from_unix((const struct sockaddr *)raw, raw_len, &peer, &peer_len) != 0) {
        peer_len = wildcard_addr(family ? family : AF_INET, &peer);
    }
    fit_family(family ? family : AF_INET, &peer, &peer_len);
    fd_record_peer(client, (struct sockaddr *)&peer, peer_len);
    if (sa && len) {
        /* Truncate to the caller's buffer and report the true length, as
           accept(2) does on Linux. */
        socklen_t capacity = *len;
        socklen_t copy = peer_len < capacity ? peer_len : capacity;
        memcpy(sa, &peer, copy);
        *len = peer_len;
    }
    return client;
}

/* accept(2) and accept4(2): a client of another user is closed, and the
   call takes the next one (a non-blocking listener then gives EAGAIN). */
int accept(int fd, struct sockaddr *sa, socklen_t *len) {
    ensure_init();
    if (!fd_is_virtual(fd)) return r_accept(fd, sa, len);
    for (;;) {
        struct sockaddr_storage raw;
        socklen_t raw_len = sizeof raw;
        int client = r_accept(fd, (struct sockaddr *)&raw, &raw_len);
        if (client >= 0 && !peer_is_own(client)) {
            logf_("accept: closed a client of another user\n");
            r_close(client);
            continue;
        }
        return finish_accept(fd, client, &raw, raw_len, sa, len);
    }
}

int accept4(int fd, struct sockaddr *sa, socklen_t *len, int flags) {
    ensure_init();
    if (!fd_is_virtual(fd)) return r_accept4(fd, sa, len, flags);
    for (;;) {
        struct sockaddr_storage raw;
        socklen_t raw_len = sizeof raw;
        int client = r_accept4(fd, (struct sockaddr *)&raw, &raw_len, flags);
        if (client >= 0 && !peer_is_own(client)) {
            logf_("accept: closed a client of another user\n");
            r_close(client);
            continue;
        }
        return finish_accept(fd, client, &raw, raw_len, sa, len);
    }
}

/* The address an un-bound or unconnected socket reports.

   getsockname on a socket that has not been bound answers with the wildcard
   address of its own family and port 0, which is what an AF_INET socket says.
   getpeername on a socket that is not connected answers ENOTCONN, not a
   wildcard: Linux has nothing to report there, and inventing an address makes
   a program believe it has a peer it does not have. */
static socklen_t wildcard_addr(int family, struct sockaddr_storage *out) {
    memset(out, 0, sizeof *out);
    if (family == AF_INET6) {
        struct sockaddr_in6 *s6 = (struct sockaddr_in6 *)out;
        s6->sin6_family = AF_INET6;
        return sizeof *s6;
    }
    struct sockaddr_in *s4 = (struct sockaddr_in *)out;
    s4->sin_family = AF_INET;
    return sizeof *s4;
}

int getpeername(int fd, struct sockaddr *sa, socklen_t *len) {
    ensure_init();
    if (fd_netlink_peer(fd, NULL) >= 0) {
        struct sockaddr_storage k;
        socklen_t k_len = sizeof k;
        nl_kernel_name(&k, &k_len);
        return copy_sockaddr(sa, len, &k, k_len);
    }
    if (!fd_is_virtual(fd)) return r_getpeername(fd, sa, len);
    pthread_mutex_lock(&fd_lock);
    struct sockaddr_storage peer;
    socklen_t peer_len = 0;
    if ((size_t)fd < fd_table_len) {
        peer = fd_table[fd].peer;
        peer_len = fd_table[fd].peer_len;
    }
    pthread_mutex_unlock(&fd_lock);
    if (peer_len == 0) {
        errno = ENOTCONN;
        return -1;
    }
    return copy_sockaddr(sa, len, &peer, peer_len);
}

int getsockname(int fd, struct sockaddr *sa, socklen_t *len) {
    ensure_init();
    if (fd_netlink_peer(fd, NULL) >= 0) return nl_getsockname(fd, sa, len);
    if (!fd_is_virtual(fd)) return r_getsockname(fd, sa, len);
    pthread_mutex_lock(&fd_lock);
    struct sockaddr_storage local;
    socklen_t local_len = 0;
    int family = 0;
    if ((size_t)fd < fd_table_len) {
        local = fd_table[fd].local;
        local_len = fd_table[fd].local_len;
        family = fd_table[fd].family;
    }
    pthread_mutex_unlock(&fd_lock);
    if (local_len == 0) {
        struct sockaddr_storage any;
        socklen_t any_len = wildcard_addr(family ? family : AF_INET, &any);
        return copy_sockaddr(sa, len, &any, any_len);
    }
    return copy_sockaddr(sa, len, &local, local_len);
}

int close(int fd) {
    ensure_init();
    int peer = fd_netlink_peer(fd, NULL);
    int rc = r_close(fd);
    if (rc == 0) fd_clear(fd);
    if (rc == 0 && peer >= 0) nl_release(peer);
    return rc;
}

/* dup(2) and friends must resolve the real symbol and read cfg.max_fds before
   fd_copy runs, or a program that duplicates a descriptor before its first
   socket call reads an unparsed configuration and silently loses virtual
   status on the copy. */
int dup(int oldfd) {
    ensure_init();
    int fd = r_dup(oldfd);
    if (fd >= 0) fd_copy(oldfd, fd);
    return fd;
}

int dup2(int oldfd, int newfd) {
    ensure_init();
    int fd = r_dup2(oldfd, newfd);
    if (fd >= 0) fd_copy(oldfd, newfd);
    return fd;
}

int dup3(int oldfd, int newfd, int flags) {
    ensure_init();
    int fd = r_dup3(oldfd, newfd, flags);
    if (fd >= 0) fd_copy(oldfd, newfd);
    return fd;
}

/* fcntl(2) is variadic and its commands take two different kinds of third
   argument: an int for the flag setters, a pointer for the out-parameters.
   Reading a va_arg of the wrong type is undefined behaviour, and reading one at
   all for a command called with two arguments is undefined behaviour too. So
   each group reads its own type, and the argument-free group reads nothing.

   The first version of this shim read a single `void *` unconditionally. On
   x86-64 SysV that happens to work, because the third argument register holds
   whatever the caller left there and the kernel ignores it for F_GETFD, but it
   is UB at every optimisation level and would break on a target whose variadic
   save area is only filled for arguments actually passed. */
int fcntl(int fd, int cmd, ...) {
    ensure_init();
    va_list ap;
    va_start(ap, cmd);

    int rc;
    switch (cmd) {
    /* No third argument. */
    case F_GETFD:
    case F_GETFL:
    case F_GETOWN:
    case F_GETSIG:
    case F_GETLEASE:
        rc = r_fcntl(fd, cmd);
        va_end(ap);
        return rc;

    /* Third argument is an int. */
    case F_SETFD:
    case F_SETFL:
    case F_SETOWN:
    case F_SETSIG:
    case F_SETLEASE:
    case F_SETPIPE_SZ:
    case F_NOTIFY:
    case F_DUPFD:
    case F_DUPFD_CLOEXEC: {
        int arg = va_arg(ap, int);
        va_end(ap);
        rc = r_fcntl(fd, cmd, arg);
        break;
    }

    /* Third argument is a pointer the kernel reads or writes through: the
       record locks (struct flock, T-390) and the owner (struct f_owner_ex). */
    case F_GETLK:
    case F_SETLK:
    case F_SETLKW:
#ifdef F_OFD_GETLK
    case F_OFD_GETLK:
    case F_OFD_SETLK:
    case F_OFD_SETLKW:
#endif
#ifdef F_GETOWN_EX
    case F_GETOWN_EX:
    case F_SETOWN_EX:
#endif
    case F_GETPIPE_SZ:
    case F_GET_SEALS:
    case F_GET_RW_HINT:
    case F_SET_RW_HINT:
    case F_ADD_SEALS: {
        void *arg = va_arg(ap, void *);
        va_end(ap);
        rc = r_fcntl(fd, cmd, arg);
        break;
    }

    default: {
        /* A command this shim does not know. Forward it as an int, which is
           the shape of every remaining Linux command; a command with a
           different shape would be reported by the kernel rather than ignored,
           because EBADF/EINVAL still come back. */
        int arg = va_arg(ap, int);
        va_end(ap);
        rc = r_fcntl(fd, cmd, arg);
        break;
    }
    }

    if (rc >= 0 && (cmd == F_DUPFD || cmd == F_DUPFD_CLOEXEC)) fd_copy(fd, rc);
    return rc;
}

ssize_t sendto(int fd, const void *buf, size_t n, int flags,
               const struct sockaddr *sa, socklen_t len) {
    ensure_init();
    uint32_t pid = 0;
    int peer = fd_netlink_peer(fd, &pid);
    if (peer >= 0) {
        ssize_t rc = r_sendto(fd, buf, n, flags | MSG_NOSIGNAL, NULL, 0);
        if (rc >= 0) nl_pump(peer, pid);
        return rc;
    }
    if (!fd_is_virtual(fd) || !sa) {
        return r_sendto(fd, buf, n, flags, sa, len);
    }
    struct sockaddr_un un;
    socklen_t un_len;
    if (to_abstract(sa, len, &un, &un_len) < 0) return -1;
    logf_("sendto -> %s\n", un.sun_path + 1);
    return r_sendto(fd, buf, n, flags, (struct sockaddr *)&un, un_len);
}

/* A datagram socket of the shim takes the same user only, as the
   stream sockets (issue 308) and the switch do. Before the real receive,
   look at the next datagram (MSG_PEEK) and drop it while its sender
   (SCM_CREDENTIALS of SO_PASSCRED) is another user. Two threads that
   receive on one socket at the same time can make this drop a datagram of
   the same user. */
static int fd_is_dgram(int fd) {
    int result = 0;
    pthread_mutex_lock(&fd_lock);
    if (fd >= 0 && (size_t)fd < fd_table_len) result = fd_table[fd].is_virtual && fd_table[fd].is_dgram;
    pthread_mutex_unlock(&fd_lock);
    return result;
}

static void drop_other_users(int fd, int flags) {
    if (!fd_is_dgram(fd)) return;
    for (;;) {
        union {
            char buf[CMSG_SPACE(sizeof(struct ucred))];
            struct cmsghdr align;
        } control;
        struct msghdr m;
        memset(&m, 0, sizeof m);
        m.msg_control = control.buf;
        m.msg_controllen = sizeof control.buf;
        int saved = errno;
        ssize_t rc = r_recvmsg(fd, &m, MSG_PEEK | (flags & MSG_DONTWAIT));
        errno = saved;
        if (rc < 0) return; /* the real call gives the error */
        struct cmsghdr *c = CMSG_FIRSTHDR(&m);
        if (!c || c->cmsg_level != SOL_SOCKET || c->cmsg_type != SCM_CREDENTIALS) return;
        struct ucred cred;
        memcpy(&cred, CMSG_DATA(c), sizeof cred);
        if (cred.uid == geteuid()) return;
        struct msghdr drop;
        memset(&drop, 0, sizeof drop);
        r_recvmsg(fd, &drop, flags & MSG_DONTWAIT);
        errno = saved;
        fprintf(stderr, "podnet-vnet: dropped a datagram of user %u (the shim takes the same user only)\n",
                (unsigned)cred.uid);
    }
}

ssize_t recvfrom(int fd, void *buf, size_t n, int flags,
                 struct sockaddr *sa, socklen_t *len) {
    ensure_init();
    uint32_t pid = 0;
    int peer = fd_netlink_peer(fd, &pid);
    if (peer >= 0) {
        nl_pump(peer, pid);
        ssize_t rc = r_recvfrom(fd, buf, n, flags, NULL, NULL);
        if (rc >= 0 && sa && len) nl_kernel_name(sa, len);
        return rc;
    }
    drop_other_users(fd, flags);
    if (!fd_is_virtual(fd) || !sa || !len) {
        return r_recvfrom(fd, buf, n, flags, sa, len);
    }
    struct sockaddr_storage src;
    socklen_t src_len = sizeof src;
    ssize_t rc = r_recvfrom(fd, buf, n, flags, (struct sockaddr *)&src, &src_len);
    if (rc < 0) return rc;
    struct sockaddr_storage logical;
    socklen_t logical_len = 0;
    if (peer_from_unix((struct sockaddr *)&src, src_len, &logical, &logical_len) == 0) {
        fit_family(fd_family(fd), &logical, &logical_len);
        if (*len >= logical_len) {
            memcpy(sa, &logical, logical_len);
            *len = logical_len;
        } else {
            *len = logical_len;
        }
    } else {
        pthread_mutex_lock(&fd_lock);
        socklen_t peer_len = (size_t)fd < fd_table_len ? fd_table[fd].peer_len : 0;
        struct sockaddr_storage peer;
        if (peer_len) memcpy(&peer, &fd_table[fd].peer, sizeof peer);
        pthread_mutex_unlock(&fd_lock);
        if (peer_len && *len >= peer_len) {
            memcpy(sa, &peer, peer_len);
            *len = peer_len;
        } else {
            *len = 0;
        }
    }
    return rc;
}

ssize_t sendmsg(int fd, const struct msghdr *msg, int flags) {
    ensure_init();
    uint32_t pid = 0;
    int peer = fd_netlink_peer(fd, &pid);
    if (peer >= 0 && msg) return nl_sendmsg(fd, peer, pid, msg, flags);
    if (!fd_is_virtual(fd) || !msg || !msg->msg_name) {
        return r_sendmsg(fd, msg, flags);
    }
    struct sockaddr_un un;
    socklen_t un_len;
    if (to_abstract((struct sockaddr *)msg->msg_name, msg->msg_namelen, &un, &un_len) < 0) return -1;
    struct msghdr copy = *msg;
    copy.msg_name = &un;
    copy.msg_namelen = un_len;
    logf_("sendmsg -> %s\n", un.sun_path + 1);
    return r_sendmsg(fd, &copy, flags);
}

ssize_t recvmsg(int fd, struct msghdr *msg, int flags) {
    ensure_init();
    uint32_t pid = 0;
    int peer = fd_netlink_peer(fd, &pid);
    if (peer >= 0 && msg) return nl_recvmsg(fd, peer, pid, msg, flags);
    drop_other_users(fd, flags);
    if (!fd_is_virtual(fd) || !msg || !msg->msg_name) {
        return r_recvmsg(fd, msg, flags);
    }
    struct sockaddr_storage src;
    struct msghdr copy = *msg;
    copy.msg_name = &src;
    copy.msg_namelen = sizeof src;
    ssize_t rc = r_recvmsg(fd, &copy, flags);
    if (rc < 0) return rc;
    struct sockaddr_storage logical;
    socklen_t logical_len = 0;
    if (peer_from_unix((struct sockaddr *)&src, copy.msg_namelen, &logical, &logical_len) == 0) {
        fit_family(fd_family(fd), &logical, &logical_len);
        socklen_t capacity = msg->msg_namelen;
        memcpy(msg->msg_name, &logical, logical_len < capacity ? logical_len : capacity);
        msg->msg_namelen = logical_len;
    } else {
        msg->msg_namelen = 0;
    }
    msg->msg_flags = copy.msg_flags;
    return rc;
}

ssize_t send(int fd, const void *buf, size_t n, int flags) {
    ensure_init();
    uint32_t pid = 0;
    int peer = fd_netlink_peer(fd, &pid);
    if (peer < 0) return r_send(fd, buf, n, flags);
    ssize_t rc = r_send(fd, buf, n, flags | MSG_NOSIGNAL);
    if (rc >= 0) nl_pump(peer, pid);
    return rc;
}

ssize_t recv(int fd, void *buf, size_t n, int flags) {
    ensure_init();
    uint32_t pid = 0;
    int peer = fd_netlink_peer(fd, &pid);
    if (peer >= 0) nl_pump(peer, pid);
    return r_recv(fd, buf, n, flags);
}

int setsockopt(int fd, int level, int optname, const void *optval, socklen_t optlen) {
    ensure_init();
    if (level == SOL_NETLINK && fd_netlink_peer(fd, NULL) >= 0) return 0;
    if (!fd_is_virtual(fd)) {
        return r_setsockopt(fd, level, optname, optval, optlen);
    }
    /* The underlying fd is AF_UNIX, so IP/TCP options are meaningless to the
     * kernel. Record the ones that matter and report success. */
    if (level == IPPROTO_TCP || level == IPPROTO_IP || level == IPPROTO_IPV6) {
        pthread_mutex_lock(&fd_lock);
        if ((size_t)fd < fd_table_len && optval && optlen >= sizeof(int)) {
            int value = *(const int *)optval;
            if (level == IPPROTO_TCP && optname == TCP_NODELAY) fd_table[fd].tcp_nodelay = value;
            else if (level == IPPROTO_TCP && optname == TCP_KEEPIDLE) fd_table[fd].tcp_keepalive = value;
        }
        pthread_mutex_unlock(&fd_lock);
        logf_("setsockopt fd=%d level=%d optname=%d swallowed\n", fd, level, optname);
        return 0;
    }
    if (level == SOL_SOCKET && optname == SO_REUSEADDR) {
        pthread_mutex_lock(&fd_lock);
        if ((size_t)fd < fd_table_len && optval && optlen >= sizeof(int)) {
            fd_table[fd].so_reuseaddr = *(const int *)optval;
        }
        pthread_mutex_unlock(&fd_lock);
    }
    return r_setsockopt(fd, level, optname, optval, optlen);
}

int getsockopt(int fd, int level, int optname, void *optval, socklen_t *optlen) {
    ensure_init();
    if (level == SOL_NETLINK && fd_netlink_peer(fd, NULL) >= 0) {
        if (!optval || !optlen || *optlen < sizeof(int)) {
            errno = EINVAL;
            return -1;
        }
        *(int *)optval = 0;
        *optlen = sizeof(int);
        return 0;
    }
    if (fd_is_virtual(fd) && (level == IPPROTO_TCP || level == IPPROTO_IP || level == IPPROTO_IPV6)) {
        if (!optval || !optlen || *optlen < sizeof(int)) {
            errno = EINVAL;
            return -1;
        }
        int value = 0;
        pthread_mutex_lock(&fd_lock);
        if ((size_t)fd < fd_table_len) {
            if (level == IPPROTO_TCP && optname == TCP_NODELAY) value = fd_table[fd].tcp_nodelay;
            else if (level == IPPROTO_TCP && optname == TCP_KEEPIDLE) value = fd_table[fd].tcp_keepalive;
        }
        pthread_mutex_unlock(&fd_lock);
        *(int *)optval = value;
        *optlen = sizeof(int);
        return 0;
    }
    return r_getsockopt(fd, level, optname, optval, optlen);
}

/* Intercept name resolution for names the virtual network owns, by rewriting
 * the node into the mapped address and letting the real resolver parse it. */
static const char *hosts_lookup(const char *name) {
    if (!name) return NULL;
    for (size_t i = 0; i < cfg.hosts_len; i++) {
        if (strcasecmp(cfg.hosts[i].name, name) == 0) return cfg.hosts[i].addr;
    }
    return NULL;
}

int getaddrinfo(const char *node, const char *service,
                const struct addrinfo *hints, struct addrinfo **res) {
    ensure_init();
    const char *mapped = hosts_lookup(node);
    if (mapped) {
        logf_("getaddrinfo %s -> %s\n", node, mapped);
        return r_getaddrinfo(mapped, service, hints, res);
    }
    return r_getaddrinfo(node, service, hints, res);
}
