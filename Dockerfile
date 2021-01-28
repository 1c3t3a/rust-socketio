FROM node:10.23.1-alpine
WORKDIR ./testdir
COPY ./example/package.json ./
RUN npm install socket.io 
RUN npm install socket.io-client 
COPY ./example/socket-io.js ./
EXPOSE 4200
CMD [ "node", "socket-io.js" ]
