# Tiny, deterministic build. The tag exists so Dependabot has a release stream to
# follow; the digest is what the build uses. The base is pinned by digest and pre-pulled, so the
# build does no network I/O; the single RUN layer keeps the work fixed across
# iterations.
FROM docker.io/library/alpine:3.24@sha256:3f4d6b32e26703c6082839234f43769d5046b3ac7c910033911d1e71d8cd630a
RUN echo "benchmark build layer" > /benchmark-marker
