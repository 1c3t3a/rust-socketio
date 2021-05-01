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
});
