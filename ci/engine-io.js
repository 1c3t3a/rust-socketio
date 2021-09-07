/**
 * This is an example server, used to test the current code.
 */
const engine = require('engine.io');
const http = require('http').createServer().listen(4201);
// the engine.io client runs on port 4201
//TODO: Add engineio servers that refuses upgrades (one http one https) for full coverage
const server = engine.attach(http);

console.log("Started")
server.on('connection', socket => {
    console.log("Connected");

    socket.on('message', message => {
        if (message !== undefined) {
            console.log(message.toString());
            if (message == "respond") {
                socket.send("Roger Roger");
            }
        } else {
            console.log("empty message received")
        }
    });

    socket.on('heartbeat', () => {
        console.log("heartbeat");
    });

    socket.on('error', message => {
        // Notify the client if there is an error so it's tests will fail
        socket.send("ERROR: Received error")
        console.log(message.toString());
    });

    socket.on('close', () => {
        console.log("Close");
        socket.close();
    });

    socket.send('hello client');
});
