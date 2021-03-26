echo "Starting test environment"
DEBUG=* node engine-io.js &
status=$?
if [ $status -ne 0 ]; then
  echo "Failed to start engine.io: $status"
  exit $status
fi
echo "Successfully started engine.io instance"

node socket-io.js &
status=$?
if [ $status -ne 0 ]; then
  echo "Failed to start socket.io: $status"
  exit $status
fi
echo "Successfully started socket.io instance"

while sleep 60; do
  ps aux |grep socket |grep -q -v grep
  PROCESS_1_STATUS=$?
  ps aux |grep engine-io.js |grep -q -v grep
  PROCESS_2_STATUS=$?
  # If the greps above find anything, they exit with 0 status
  # If they are not both 0, then something is wrong
  if [ $PROCESS_1_STATUS -ne 0 -o $PROCESS_2_STATUS -ne 0 ]; then
    echo "One of the processes has already exited."
    exit 1
  fi
done
