FROM rust:1-slim-buster AS base

LABEL maintainer="Narayan Newton <nnewton@tag1consulting.com>"
LABEL org.label-schema.vendor="Tag1 Consulting" \
  org.label-schema.url="https://github.com/tag1consulting/goose" \
  org.label-schema.name="Goose" \
  org.label-schema.version="mainline" \
  org.label-schema.vcs-url="github.com:tag1consulting/goose.git" \
  org.label-schema.docker.schema-version="1.0"

ENV GOOSE_EXAMPLE=umami \
    GOOSE_FEATURES="gaggle"

ARG DEBIAN_FRONTEND=noninteractive

COPY . /build
WORKDIR ./build

RUN apt-get update && \
  apt-get install -y libssl-dev gcc pkg-config cmake && \
  apt-get clean && \
  rm -rf /var/lib/apt/lists/*

RUN cargo build --features "${GOOSE_FEATURES}" --release --example "${GOOSE_EXAMPLE}"
RUN chmod +x ./docker-entrypoint.sh

EXPOSE 5115
ENTRYPOINT ["./docker-entrypoint.sh"]
