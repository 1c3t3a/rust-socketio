#!/bin/sh
cd $(dirname $0)

if [ "$1" = "" ] || [ "$2" = "" ]
then
    echo "Usage: keygen.sh [DOMAIN] [IP]"
    exit 1
fi

DOMAIN="$1"
IP="$2"
CA_NAME=${CA_NAME:-"rust-socketio-dev"}

mkdir cert || true
cd cert
# Credit https://scriptcrunch.com/create-ca-tls-ssl-certificates-keys/

if [ ! -f ca.key ]
then
    echo "Generating CA key"
    openssl genrsa -out ca.key 4096
fi

if [ ! -f "ca.crt" ]
then
    echo "Generating CA cert"
    openssl req -x509 -new -nodes -key ca.key -subj "/CN=${CA_NAME}/C=??/L=Varius" -out ca.crt
fi

if [ ! -f "server.key" ]
then
    echo "Generating server key"
    openssl genrsa -out server.key 4096
fi

if [ ! -f "csr.conf" ]
then
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
O = ${DOMAIN}
OU = ${DOMAIN}
CN = ${DOMAIN}

[ req_ext ]
subjectAltName = @alt_names

[ alt_names ]
DNS.1 = ${DOMAIN}
DNS.2 = localhost
IP.1 = ${IP}
""" > csr.conf
fi

if [ ! -f "server.csr" ]
then
    echo "Generating server signing request"
    openssl req -new -key server.key -out server.csr -config csr.conf
fi

if [ ! -f "server.crt" ]
then
    echo "Generating signed server certifcicate"
    openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -out server.crt -extfile csr.conf
fi
