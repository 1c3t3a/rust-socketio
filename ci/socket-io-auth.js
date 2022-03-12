const server = require('http').createServer();
const io = require('socket.io')(server);

console.log('Started');
var callback = client => {
    console.log('Connected!');;
    client.on('test', function (arg, ack) {
        console.log('Ack received')
        if (ack) {
            const payload = client.handshake.auth.password === '123' ? '456' : '789'
            ack(payload);
        }
    });
};
io.on('connection', callback);
io.of('/admin').on('connection', callback);
// the socket.io client runs on port 4201
server.listen(4204);
