let createServer = require("http").createServer;
let server = createServer();
const io = require("socket.io")(server);
const port = 4205;
const timeout = 2000;

console.log("Started");
var callback = (client) => {
  const headers = client.request.headers;
  console.log("headers", headers);
  const message = headers.message_back || "test";

  console.log("Connected!");
  client.emit("message", message);
  client.on("restart_server", () => {
    console.log("will restart in ", timeout, "ms");
    io.close();
    setTimeout(() => {
      server = createServer();
      server.listen(port);
      io.attach(server);
      console.log("do restart");
    }, timeout);
  });
};
io.on("connection", callback);
server.listen(port);
