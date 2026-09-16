# syntax=docker/dockerfile:1
FROM rust@sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97 AS builder
WORKDIR /workspace
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/workspace/target \
    cargo build --locked --workspace --bins --jobs 2 && \
    mkdir /out && \
    cp target/debug/masonwing target/debug/masonwing-host-api target/debug/masonwing-worker \
       target/debug/masonwing-component-runner target/debug/masonwing-remote-runner /out/

FROM python@sha256:a9bee15510a364124aa24692899d269835683b883de42f7ebec8c293cf679ccb AS runtime
RUN groupadd --gid 10001 masonwing && useradd --uid 10001 --gid 10001 --no-create-home masonwing
COPY --from=builder /out/ /usr/local/bin/
USER 10001:10001
ENV MASONWING_ENV=LOCAL MASONWING_BIND_ADDR=0.0.0.0:8080 PYTHONDONTWRITEBYTECODE=1
EXPOSE 8080
CMD ["masonwing-host-api"]
