const engine = require('engine.io');
const server = engine.listen(4202, {
    transports: ["polling"]
  });

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

