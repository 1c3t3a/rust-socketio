#!/bin/sh
mkdir cert || true
cd cert
# Credit https://scriptcrunch.com/create-ca-tls-ssl-certificates-keys/

echo "Generating CA key"
openssl genrsa -out ca.key 4096

echo "Generating CA cert"
openssl req -x509 -new -nodes -key ca.key -subj "/CN=rust-socketio-dev/C=??/L=Varius" -out ca.crt

echo "Generating server key"
openssl genrsa -out server.key 4096

echo """
[ req ]
default_bits = 4096
prompt = no
default_md = sha256
req_extensions = req_ext
distinguished_name = dn

[ dn ]
C = ??
ST = Varius
L = Varius
O = node-engine-io-secure
OU = node-engine-io-secure
CN = node-engine-io-secure

[ req_ext ]
subjectAltName = @alt_names

[ alt_names ]
DNS.1 = localhost
IP.1 = 127.0.0.1
""" > csr.conf

echo "Generating server signing request"
openssl req -new -key server.key -out server.csr -config csr.conf

echo "Generating signed server certifcicate"
openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -out server.crt -extfile csr.conf
