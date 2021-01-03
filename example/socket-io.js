const server = require('http').createServer();
const io = require('socket.io')(server);

console.log("Started");
io.on('connection', client => {
    console.log("Connected!");
    client.on('test', data => { console.log(data) });
    client.on('message', data => { console.log(data) });
});
server.listen(4200);
