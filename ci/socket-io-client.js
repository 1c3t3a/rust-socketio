const { Manager } = require("socket.io-client");


const manager = new Manager("http://127.0.0.1:4209");

const socket = manager.socket("/admin");

socket.on("connect", () => {
  const engine = socket.io.engine;
  console.log("connect " + engine.transport.name); // in most cases, prints "polling"

  engine.once("upgrade", () => {
    // called when the transport is upgraded (i.e. from HTTP long-polling to WebSocket)
    console.log(engine.transport.name); // in most cases, prints "websocket"
  });

  engine.on("packet", ({ type, data }) => {
    // called for each packet received
  });

  engine.on("packetCreate", ({ type, data }) => {
    // called for each packet sent
  });

  engine.on("drain", () => {
    // called when the write buffer is drained
  });

  engine.on("close", (reason) => {
    // called when the underlying connection is closed
    console.log("close " + reason);
  });
});

socket.on("disconnect", (reason) => {
  console.log("disconnect " + reason); // false
});

