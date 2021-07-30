FROM mcr.microsoft.com/vscode/devcontainers/rust:0-1

COPY ./ci/cert/ca.crt /usr/local/share/ca-certificates/

RUN update-ca-certificates
