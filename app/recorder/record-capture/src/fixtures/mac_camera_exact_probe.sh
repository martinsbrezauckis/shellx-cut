#!/bin/sh
output=''
ignore=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) output="$2"; shift ;;
    -ignore_editlist) ignore="$2"; shift ;;
  esac
  shift
done
if [ -z "$output" ]; then
  printf '%s\n' '{"streams":[{"time_base":"1/600","duration_ts":5,"nb_read_frames":"5","nb_read_packets":"5"}],"format":{"duration":"0.008333"}}'
elif [ "$ignore" = '1' ]; then
  printf '0,1,50,K__\n1,1,100,K__\n2,1,200,___\n3,1,300,___\n4,1,400,___\n' > "$output"
else
  printf '0,1,100,K__\n1,1,200,___\n2,1,300,___\n3,1,400,_D_\n' > "$output"
fi
