const fs = require('fs');
const https = require('https');
const eio = require('engine.io');

const serverOpts = {
    key: fs.readFileSync("cert/server.key"),
    cert: fs.readFileSync("cert/server.crt"),
    ca: fs.readFileSync("cert/ca.crt"),
};

const http = https.createServer(serverOpts);

const server = eio.attach(http);

console.log("Started")
http.listen(4202, () => {
    server.on('connection', socket => {
        console.log("Connected");
    
        socket.on('message', message => {
            if (message !== undefined) {
                console.log(message.toString());
                if (message == "respond") {
                    socket.send("Roger Roger");
                } else if (message == "close") {
                    socket.close();
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
});
