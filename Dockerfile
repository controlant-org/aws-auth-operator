FROM ubuntu:20.04

COPY target/release/operator /operator

ENTRYPOINT ["/operator"]
