const server = require('http').createServer();
const io = require('socket.io')(server);

console.log('Started');
var callback = client => {
    console.log('Connected!');

    client.emit('auth', client.handshake.auth.password === '123' ? 'success' : 'failed')
};
io.on('connection', callback);
io.of('/admin').on('connection', callback);
// the socket.io client runs on port 4204
server.listen(4204);
