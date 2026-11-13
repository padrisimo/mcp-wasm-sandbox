# Build the MCP server and ship it in a minimal image.
#
#   docker build -t mcp-wasm-sandbox:bench .
#   docker run --rm -i mcp-wasm-sandbox:bench
FROM rust:1-slim AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p mcp-server

FROM debian:stable-slim
COPY --from=build /src/target/release/mcp-server /usr/local/bin/mcp-server
ENTRYPOINT ["/usr/local/bin/mcp-server"]
