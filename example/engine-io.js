/**
 * This is an example server, used to test the current code.
 */
const engine = require('engine.io');
// the engine.io client runs on port 4201
const server = engine.listen(4201);

console.log("Started")
server.on('connection', socket => {
    console.log("Connected");

    socket.on('message', message => {
        console.log(message.toString());
        if (message == "CLOSE") {
            socket.close();
        }
    });

    socket.on('ping', () => {
        console.log("Ping");
    });

    socket.on('pong', () => {
        console.log("Pong");
    });

    socket.on('error', message => {
        console.log(message.toString());
    });

    socket.on('close', () => { 
        console.log("Close");
    });

    socket.send('utf 8 string');
});


