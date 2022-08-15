let createServer = require('http').createServer;
let server = createServer();
const io = require('socket.io')(server);
const port = 4205;
const timeout = 2000;


console.log('Started');
var callback = client => {
    console.log('Connected!');
    client.on('restart_server', () => {
        console.log('will restart in ', timeout, 'ms');
        io.close();
        setTimeout(() => {
            console.log("do restart")
            server = createServer();
            server.listen(port);
            io.attach(server);
        }, timeout)
    });
};
io.on('connection', callback);
io.of('/admin').on('connection', callback);
// the socket.io client runs on port 4201
server.listen(port);
