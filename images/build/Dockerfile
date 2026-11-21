# The image that builds the static Linux binaries of podnet (release.yml). This
# line is the one place that names it; the workflow reads it. The digest is
# that of the multi-platform index (linux/amd64 and linux/arm64), so two runs
# of one commit use one compiler, on each architecture.
FROM docker.io/library/rust:1-alpine@sha256:8d0e82abfdf9a38071e07cabc5afba4ddbe077631a80e4e742a760c38beadc33
