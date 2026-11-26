# shellcheck shell=sh
# docs/offline.md: the reference list. The set-up writes the inputs of each
# line: report.txt, ./site, the base bundle ./old (the site before a change),
# two captures, a file of secrets, and two replica logs with no record.
# `discover listen` waits 10 s, and the debug build hashes the code slowly.
# The block is n.a. on a host that sends no UDP to the loopback (discover).
# shellcheck disable=SC2034
LIMIT=60

setup() {
    doc_need_udp
    echo 'a report' > report.txt
    mkdir site
    echo 'the first file' > site/a.txt
    echo 'the second file' > site/b.txt
    podnet offline bundle build --out ./old --name field --file ./site:site > /dev/null
    echo 'a change' >> site/b.txt
    printf 'pod-offline capture 1\n1000 in 0 68656c6c6f\n' > a.cap
    printf 'pod-offline capture 1\n2000 out 0 6120736563726574\n' > b.cap
    echo 'secret' > secrets.txt
    : > a.log
    : > b.log
}

check() {
    grep -q 'a report' taken.txt
    podnet offline bundle verify ./bundle > /dev/null
    ! grep -q 6120736563726574 all.cap
}
