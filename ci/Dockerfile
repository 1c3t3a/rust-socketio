FROM node:12.18.1
WORKDIR /test
COPY . ./
COPY start_test_server.sh ./

RUN npm install

RUN chmod u+x start_test_server.sh
CMD ./start_test_server.sh
