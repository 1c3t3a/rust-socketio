const server = require('http').createServer();
const io = require('socket.io')(server);

console.log("Started");
io.on('connection', client => {
    console.log("Connected!");
    client.on('test', data => { console.log(data) });
    client.on('message', data => { console.log(data) });
    client.on('test', function (arg, ack) {
        console.log('Ack received')
        if (ack) {
            ack('woot');
        }
      });
    client.emit("test", "Hello Wld");
});
// the socket.io client runs on port 4201
server.listen(4200);
