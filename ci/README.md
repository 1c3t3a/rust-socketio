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

To run the tests locally, simply use `cargo test`, or the various Make targets in the `Makefile`. For example
`make pipeline` runs all tests, but also `clippy` and `rustfmt`.

As some tests depend on a running engine.io/socket.io server, you will need to provide those locally, too. See the
sections below for multiple options to do this.

You will also need to create a self-signed certificate for the secure engine.io server. This can be done with the
helper script `/ci/keygen.sh`. Please make sure to also import the generated ca and server certificates into your
system (i.e. Keychain Access for MacOS, /usr/local/share/ca-certificates/ for linux) and make sure they are "always
trusted".

## Running server processes manually via nodejs

If you'd like to run the full test suite locally, you need to run the five server instances as well (see files in `ci/`
folder). You could do this manually by running them all with node:

```
node engine-io.js 
node engine-io-polling.js
node engine-io-secure.js
node socket-io.js
node socket-io-auth.js 
```

If you'd like to see debug log as well, export this environment variable beforehand:

```
export DEBUG=*
```

You will need to have the two node packages `socket.io` and `engine.io` installed, if this is not the case, fetch them
via:

```
npm install socket.io engine.io
```

## Running server processes in a Docker container

As running all the node scripts manually is pretty tedious, you can also use a prepared docker container, which can be
built with the Dockerfile located in the `ci/` folder:

```
docker build -t test_suite:latest ci
```

Then you can run the container and forward all the needed ports with the following command:

```
docker run -d --name test_suite -p 4200:4200 -p 4201:4201 -p 4202:4202 -p 4203:4203 -p 4204:4204 test_suite:latest
```

The docker container runs a shell script that starts the two servers in the background and checks if the processes are
still alive.

## Using the Visual Studio Code devcontainer

If you are using Visual Studio Code, the easiest method to get up and running would be to simply use the devcontainer
prepared in the `.devcontainer/` directory. This will also launch the needed server processes and set up networking etc.
Please refer to the vscode [documentation](https://code.visualstudio.com/docs/remote/containers) for more information
on how to use devcontainers.

# Polling vs. Websockets

The underlying engine.io protocol provides two mechanisms for transporting: polling and websockets. In order to test both in the pipeline, the two servers are configured differently. The socket.io test suite always upgrades to websockets as fast as possible while one of the engine.io suites just uses long-polling, the other one uses websockets but is reachable via `https://` and `wss://`. This assures that both the websocket connection code and the long-polling code gets tested (as seen on codecov.io). Keep that in mind while expanding the tests.
