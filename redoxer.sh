#!/bin/sh

set -e

old_path="file:/bin/ipcd"
new_path="target/x86_64-unknown-redox/debug/ipcd"
if [ -e "${new_path}" ]
then
    mv -v "${new_path}" "${old_path}"
    shutdown --reboot
fi

while [ "$#" != "0" ]
do
    example="$1"
    shift

    echo "# ${example} #"
    "target/x86_64-unknown-redox/debug/examples/${example}"
done
