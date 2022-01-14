FROM ubuntu:20.04

RUN apt-get update && apt-get install -y libssl-dev ca-certificates

COPY target/release/operator /operator

ENTRYPOINT ["/operator"]
