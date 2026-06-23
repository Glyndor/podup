# Tiny, deterministic build. The base is pinned by digest and pre-pulled, so the
# build does no network I/O; the single RUN layer keeps the work fixed across
# iterations.
FROM docker.io/library/alpine@sha256:28bd5fe8b56d1bd048e5babf5b10710ebe0bae67db86916198a6eec434943f8b
RUN echo "benchmark build layer" > /benchmark-marker
