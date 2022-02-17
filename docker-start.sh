#!/bin/zsh

CANDIDATE=$(ifconfig en0 inet| grep 'inet '|awk '{print $2}')

echo $CANDIDATE

docker run --rm --env CANDIDATE=$CANDIDATE \
  -p 1935:1935 -p 8080:8080 -p 1985:1985 -p 8000:8000/udp \
  srs-sha256 \
  objs/srs -c conf/rtc.conf

docker run --rm -p 1989:1989 registry.cn-hangzhou.aliyuncs.com/ossrs/signaling:1

CANDIDATE=$(ifconfig en0 inet| grep 'inet '|awk '{print $2}')

echo $CANDIDATE

docker run --rm -p 80:80 -p 443:443 registry.cn-hangzhou.aliyuncs.com/ossrs/httpx:v1.0.2 \
    ./bin/httpx-static -http 80 -https 443 -ssk ./etc/server.key -ssc ./etc/server.crt \
          -proxy http://$CANDIDATE:1989/sig -proxy http://$CANDIDATE:1985/rtc \
          -proxy http://$CANDIDATE:8080/

