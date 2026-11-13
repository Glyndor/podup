# Tiny, deterministic build. The tag exists so Dependabot has a release stream to
# follow; the digest is what the build uses. The base is pinned by digest and pre-pulled, so the
# build does no network I/O; the single RUN layer keeps the work fixed across
# iterations.
FROM docker.io/library/alpine:3.24@sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf
RUN echo "benchmark build layer" > /benchmark-marker
