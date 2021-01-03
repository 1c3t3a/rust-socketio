# Rust-socketio

An asynchronous implementation of the socketio protocol written in the Rust programming language.

## Current features

This is the first released version of the client, so it still lacks some features that the normal client would provide. First of all the underlying engine.io protocol still uses long-polling instead of websockets. This will be resolved as soon as both the request libary as well as tungsenite-websockets will bump their tokio version to 1.0.0. At the moment only reqwest is used for async long polling. In general the full engine-io protocol is implemented and most of the features concerning the 'normal' socket.io protocol work as well. Here's an overview of possible use-cases:

## Content of this repository

This repository contains a rust implementation of the socket.io protocol as well as the underlying engine.io protocol.

The details about the engine.io protocol can be found here:
    - <https://github.com/socketio/engine.io-protocol>

The specification for the socket.io protocol here:
    - <https://github.com/socketio/socket.io-protocol>

Looking at the component chart, the following parts need to be implemented:

<img src="docs/res/dependencies.jpg" width="50%"/>
