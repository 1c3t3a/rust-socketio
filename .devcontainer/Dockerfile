FROM mcr.microsoft.com/vscode/devcontainers/rust:0-1

# Install socat needed for TCP proxy
RUN apt update && apt install -y socat

COPY ./ci/cert/ca.crt /usr/local/share/ca-certificates/

RUN update-ca-certificates
