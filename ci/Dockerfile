FROM node:12.18.1
WORKDIR /test
COPY . ./
COPY start_test_server.sh ./

RUN cp cert/ca.crt /usr/local/share/ca-certificates/ && update-ca-certificates

RUN npm install

RUN chmod u+x start_test_server.sh
CMD ./start_test_server.sh
