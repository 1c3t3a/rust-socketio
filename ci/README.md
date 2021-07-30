# How the CI pipelines are set up

This document explains how the CI pipeline is set up. Generally, the pipeline runs on ever push to the `main` branch or on a pull request.
There are three different pipelines:

  *  Build and Codestyle: Tries to build the application and checks the code quality and formatting through `cargo clippy` and `cargo fmt`. 
     If you'd like to trigger this manually, run `make checks` in the project root.
  *  Build and test: Builds the code and kicks of a docker container containing the socket.io and engine.io servers. Then the tests run against the servers. The servers code should not be changed as the clients' tests assume certain events to fire e.g. an ack gets acked or a certain namespace exists. Two servers are started:
      * An engine.io server with some callbacks that send normal string data.
      * A _safe_ engine.io server with some callbacks that send normal string data. Generate keys for TLS with `./ci/keygen.sh localhost 127.0.0.1`. This server is used for tests using `wss://` and `https://`.
      * A socket.io server which sends string and binary data, handles acks, etc.
  * Generate coverage: This action acts like the `Build and test` action, but generates a coverage report as well. Afterward the coverage report is uploaded to codecov.io.
    This action also collects the docker server logs and uploads them as an artifact.

# How to run the tests locally

If you'd like to run the full test suite locally, you need to run the three server instances as well. You could do this manually by running them directly with node:

```
node engine-io.js / socket-io.js / engine-io-secure.js
```
or if you'd like to see debug log as well:
```
DEBUG=* node engine-io.js / socket-io.js / engine-io-secure.jss
```

You need to have the two dependencies socket.io and engine.io installed, if this is not the case, fetch them via `npm install`.

If you don't want to run the servers locally, you could also build and run the docker container via:

```
docker build -t test_suite:latest .
docker run -d --name test_suite -p 4200:4200 -p 4201:4201 -p 4202:4202 test_suite:latest
```
The docker container runs a shell script that starts the two servers in the background and checks if the processes are still alive.

As soon as the servers are running, you can start the test via `make pipeline`, which will execute every tests that's run in the whole pipeline.

# Polling vs. Websockets

The underlying engine.io protocol provides two mechanisms for transporting: polling and websockets. In order to test both in the pipeline, the two servers are configured differently. The socket.io test suite always upgrades to websockets as fast as possible while one of the engine.io suites just uses long-polling, the other one uses websockets but is reachable via `https://` and `wss://`. This assures that both the websocket connection code and the long-polling code gets tested (as seen on codecov.io). Keep that in mind while expanding the tests.
