# Rust-socketio
An implementation of the socketio protocol written in the rust programming language.

## Purpose of this project
The main goal of this project is to build a Socketio client for Rust.
Therefore it's necessary to implement the underlying engine.io protocol, which is basically wrapped by the socketio
protocol. All further information could be found here: https://socket.io/docs/v2/internals/index.html.

Looking at the component chart, the following parts need to be implemented:
![](docs/res/dependencies.pdf)
