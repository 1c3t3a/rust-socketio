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

    client.on('binary', data => {
        var bufView = new Uint8Array(data);
        console.log("Yehaa binary payload!");
        for (elem in bufView) {
            console.log(elem);
        }
    });

    client.on('binary', function (arg, ack) {
        console.log('Ack received, answer with binary')
        if (ack) {
            ack(Buffer.from([1, 2, 3]));
        }
    });
    client.emit("test", "Hello Wld");
    client.emit("test", Buffer.from([1, 2, 3]));
});
// the socket.io client runs on port 4201
server.listen(4200);
