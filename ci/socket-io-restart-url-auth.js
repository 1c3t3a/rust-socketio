const { Socket } = require("socket.io");

let createServer = require("http").createServer;
let server = createServer();
const io = require("socket.io")(server);
const port = 4206;
const timeout = 200;

function isTimestampValid(timestampStr) {
  if (timestampStr === undefined) return false;
  const timestamp = parseInt(timestampStr);
  if (isNaN(timestamp)) return false;

  const diff = Date.now() - timestamp;
  return Math.abs(diff) <= 1000;
}

console.log("Started");
var callback = (client) => {
  const timestamp = client.request._query.timestamp;
  console.log("Connected, timestamp:", timestamp);
  if (!isTimestampValid(timestamp)) {
    console.log("Invalid timestamp!");
    client.disconnect();
    return;
  }
  client.emit("message", "test");
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
