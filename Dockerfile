FROM rust:1-slim-buster AS manager

LABEL maintainer="Narayan Newton <nnewton@tag1consulting.com>"
LABEL org.label-schema.vendor="Tag1 Consulting" \
  org.label-schema.url="https://github.com/tag1consulting/goose" \
  org.label-schema.name="Goose" \
  org.label-schema.version="0.10.13" \
  org.label-schema.vcs-url="github.com:tag1consulting/goose.git" \
  org.label-schema.docker.schema-version="1.0"

ENV GOOSE_EXAMPLE=simple \
    HOST=localhost \
    USERS=1 \
    HATCH_RATE=2 \
    RUN_TIME=60 \
    MANAGER_HOST="127.0.0.1" \
    MANAGER_PORT=5115 \
    WORKERS=1 \
    OPTIONS=""

ARG DEBIAN_FRONTEND=noninteractive

COPY . /build
WORKDIR ./build
RUN apt-get update && apt-get install -y libssl-dev gcc pkg-config cmake && apt-get clean
RUN cargo run --features gaggle --example simple -- -h
CMD cargo run --features gaggle --example ${GOOSE_EXAMPLE} -- -H ${HOST} -u ${USERS} -r ${HATCH_RATE} -t ${RUN_TIME} --manager --manager-bind-port ${MANAGER_PORT} --expect-workers ${WORKERS} ${OPTIONS}

FROM manager AS worker
CMD cargo run --features gaggle --example ${GOOSE_EXAMPLE} -- --worker --manager-host ${MANAGER_HOST} --manager-port ${MANAGER_PORT} ${OPTIONS}
