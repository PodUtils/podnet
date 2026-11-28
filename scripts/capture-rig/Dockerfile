FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

# Hermetic, ultra-redundant capture rig base image:
#   - tshark: primary QUIC / HTTP/3 / HTTP/2 / TLS dissector with SSLKEYLOGFILE
#   - tcpdump: passive raw PCAP packet capture across all interfaces
#   - openssl: ephemeral per-run CA and leaf certificate signing for MITM plane
#   - python3: zero-dependency MITM/passthrough proxy, fallback dissector, redactor
#   - strace: syscall-level network fallback if a binary ignores proxy/keylog
#   - dbus + ca-certificates + curl + jq + iproute2 + nftables: client runtime support
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    dbus \
    dnsutils \
    iproute2 \
    iptables \
    jq \
    libnftables1 \
    libnss3 \
    libtss2-esys-3.0.2-0t64 \
    libtss2-tctildr0t64 \
    nftables \
    openssl \
    python3 \
    strace \
    tcpdump \
    tshark \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /rig
COPY mitm_proxy.py quic_dissect.py redact.py /rig/
COPY clients/ /rig/clients/
RUN chmod +x /rig/clients/*.sh

ENTRYPOINT ["/bin/sh"]
