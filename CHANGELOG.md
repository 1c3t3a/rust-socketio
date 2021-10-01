# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog], and this project adheres to
[Semantic Versioning]. The file is auto-generated using [Conventional Commits].

[keep a changelog]: https://keepachangelog.com/en/1.0.0/
[semantic versioning]: https://semver.org/spec/v2.0.0.html
[conventional commits]: https://www.conventionalcommits.org/en/v1.0.0-beta.4/

## Overview

* [unreleased](#unreleased)
* [`0.3.0-alpha`](#121) - _2021.09.20_
* [`0.2.4`](#024) - _2021.05.25_
* [`0.2.3`](#023) - _2021.05.24_
* [`0.2.2`](#022) - _2021.05.13
* [`0.2.1`](#021) - _2021.04.27_
* [`0.2.0`](#020) – _2021.03.13_
* [`0.1.1`](#011) – _2021.01.10_
* [`0.1.0`](#010) – _2021.01.05_

## _[Unreleased]_

### Changes

* Switch Websocket library to tungstenite-rs
    * Removed InvalidWebsocketURL
    * Changed inner type of WebSocketError to tungstenite-rs'


## <a name="030-alpha">[0.3.0-alpha] - _Refactoring_ </a>

_2021.09.20_

### Changes

* Refactored Errors
    * Renamed EmptyPacket to EmptyPacket()
    * Renamed IncompletePacket to IncompletePacket()
    * Renamed InvalidPacket to InvalidPacket()
    * Renamed Utf8Error to InvalidUtf8()
    * Renamed Base64Error to InvalidBase64
    * Renamed InvalidUrl to InvalidUrlScheme
    * Renamed ReqwestError to IncompleteResponseFromReqwest
    * Renamed HttpError to IncompleteHttp
    * Renamed HandshakeError to InvalidHandshake
    * Renamed ~ActionBeforeOpen to IllegalActionBeforeOpen()~
    * Renamed DidNotReceiveProperAck to MissingAck
    * Renamed PoisonedLockError to InvalidPoisonedLock
    * Renamed FromWebsocketError to IncompleteResponseFromWebsocket
    * Renamed FromWebsocketParseError to InvalidWebsocketURL
    * Renamed FromIoError to IncompleteIo
    * New error type InvalidUrl(UrlParseError)
    * New error type InvalidInteger(ParseIntError)
    * New error type IncompleteResponseFromEngineIo(rust_engineio::Error)
    * New error type InvalidAttachmentPacketType(u8)
    * Removed EmptyPacket
* Refactored Packet
    * Renamed encode to From<&Packet>
    * Renamed decode to TryFrom<&Bytes>
    * Renamed attachments to attachments_count
    * New struct member attachments: Option<Vec<Bytes>>
* Refactor PacketId
    * Renamed u8_to_packet_id to TryFrom<u8> for PacketId
* Refactored SocketBuilder
    * Renamed set_namespace to namespace
    * Renamed set_tls_config to tls_config
    * Renamed set_opening_header to opening_header
    * namespace returns Self rather than Result<Self>
    * opening_header accepts a Into<HeaderValue> rather than HeaderValue
* Allows for pure websocket connections
* Refactor EngineIO module

## <a name="024">[0.2.4] - _Bugfixes_ </a>

_2021.05.25_

### Changes

* Fixed a bug that prevented the client from receiving data for a message event issued on the server.

## <a name="023">[0.2.3] - _Disconnect methos on the Socket struct_ </a>

_2021.05.24_

### Changes

* Added a `disconnect` method to the `Socket` struct as requested in [#43](https://github.com/1c3t3a/rust-socketio/issues/43).

## <a name="022">[0.2.2] - _Safe websockets and custom headers_ </a>

_2021.05.13_

### Changes

* Added websocket communication over TLS when either `wss`, or `https` are specified in the URL.
* Added the ability to configure the TLS connection by providing an own `TLSConnector`.
* Added the ability to set custom headers as requested in [#35](https://github.com/1c3t3a/rust-socketio/issues/35).

## <a name="021">[0.2.1] - _Bugfixes_ </a>

_2021.04.27_

### Changes

* Corrected memory odering issues which might have become an issue on certain platforms.
* Added this CHANGELOG to keep track of all changes.
* Small stylistic changes to the codebase in general.

## <a name="020">[0.2.0] - _Fully implemented the socket.io protocol 🎉_ </a>

_2021.03.13_


### Changes

* Moved from async rust to sync rust.
* Implemented the missing protocol features.
    * Websocket as a transport layer.
    * Binary payload.
* Added a `SocketBuilder` class to easily configure a connected client.
* Added a new `Payload` type to manage the binary and string payload.


## <a name="011">[0.1.1] - _Update to tokio 1.0_</a>

_2021.01.10_

### Changes

* Bumped `tokio` to version `1.0.*`, and therefore reqwest to `0.11.*`.
* Removed all `unsafe` code.

## <a name="010">[0.1.0] - _First release of rust-socketio 🎉_</a>

_2021.01.05_

* First version of the libary written in async rust. The features included:
    * connecting to a server.
    * register callbacks for the following event types:
        * open, close, error, message
    * custom events like "foo", "on_payment", etc.
    * send json-data to the server (recommended to use serde_json as it provides safe handling of json data).
    * send json-data to the server and receive an ack with a possible message.

