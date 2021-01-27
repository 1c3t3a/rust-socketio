FROM node:10.23.1-alpine
WORKDIR /example
RUN npm install express@4
EXPOSE 4200
CMD [ "node", "socket-io.js" ]
