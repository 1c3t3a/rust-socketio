echo "Starting test environment"
DEBUG=* node engine-io.js &
status=$?
if [ $status -ne 0 ]; then
  echo "Failed to start engine.io: $status"
  exit $status
fi
echo "Successfully started engine.io instance"

DEBUG=* node engine-io-polling.js &
status=$?
if [ $status -ne 0 ]; then
  echo "Failed to start polling engine.io: $status"
  exit $status
fi
echo "Successfully started polling engine.io instance"

DEBUG=* node socket-io.js &
status=$?
if [ $status -ne 0 ]; then
  echo "Failed to start socket.io: $status"
  exit $status
fi
echo "Successfully started socket.io instance"

DEBUG=* node socket-io-auth.js &
status=$?
if [ $status -ne 0 ]; then
  echo "Failed to start socket.io auth: $status"
  exit $status
fi
echo "Successfully started socket.io auth instance"

DEBUG=* node socket-io-restart.js &
status=$?
if [ $status -ne 0 ]; then
  echo "Failed to start socket.io-restart: $status"
  exit $status
fi
echo "Successfully started socket.io-restart instance"

DEBUG=* node engine-io-secure.js &
status=$?
if [ $status -ne 0 ]; then
  echo "Failed to start secure engine.io: $status"
  exit $status
fi
echo "Successfully started secure engine.io instance"

while sleep 60; do
  ps aux | grep socket | grep -q -v grep
  PROCESS_1_STATUS=$?
  ps aux | grep engine-io.js | grep -q -v grep
  PROCESS_2_STATUS=$?
  ps aux | grep engine-io-secure.js | grep -q -v grep
  PROCESS_3_STATUS=$?
  # If the greps above find anything, they exit with 0 status
  # If they are not both 0, then something is wrong
  if [ $PROCESS_1_STATUS -ne 0 -o $PROCESS_2_STATUS -ne 0 -o $PROCESS_3_STATUS -ne 0]; then
    echo "One of the processes has already exited."
    exit 1
  fi
done
