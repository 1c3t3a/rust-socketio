# How the CI pipeline is set up

This document explains how the CI pipeline is set up. Generally, the pipeline runs on ever push to the `main` branch or on a pull request.
There are three different pipelines:

  *  Build and Codestyle: Tries to build the application and checks the code quality and formatting through `cargo clippy` and `cargo fmt`. 
     If you'd like to trigger this manually, run `make checks` in the project root.
  *  Build and test: Builds the code and kicks of a docker container containing two servers. Then the tests run against the two servers.
      * An engine.io server with some callbacks that send normal string data.
      * A socket.io server which sends string and binary data, handles acks, etc.
  * Generate coverage: This action acts like the `Build and test` action, but generates a coverage report as well. This coverage report is then uploaded to codecov.io.
    This action also collects the docker server logs and uploads them as an artifact.

# How to run the tests locally

If you'd like to run the full test suite locally, you need to run the two server instances as well. You could do this manually by running them directly with node:

```
node engine-io.js / socket-io.js
```

You need to have the two dependencies socket.io and engine.io installed, if this is not the case, fetch them via `npm install`.

If you don't want to run the servers locally, you could also build and run the docker container via:

```
docker build -t test_suite:latest .
docker run -d --name test_suite -p 4200:4200 -p 4201:4201 test_suite:latest
```

As soon as the servers are running, you can start the test via `make pipeline`, which will execute every tests that's run in the whole pipeline.
