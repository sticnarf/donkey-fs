#!/bin/sh
# Run pjdfstest from https://github.com/pjd/pjdfstest

# Create a fake block device
sudo fallocate -l 512M /opt/fake-dev0-backstore
sudo mknod /dev/fake-dev0 b 7 42
sudo losetup /dev/fake-dev0 /opt/fake-dev0-backstore

# Format and mount
sudo mkdir -p /dkfs
sudo target/debug/mkdk /dev/fake-dev0
sudo target/debug/mtdk /dev/fake-dev0 /dkfs > /tmp/dkfs.log &
sleep 1

set -e
function cleanup {
    # Unmount
    cd /
    sudo umount /dkfs
    sudo rmdir /dkfs

    # Destroy the fake block device
    sudo losetup -d /dev/fake-dev0
    sudo rm /dev/fake-dev0
    sudo rm /opt/fake-dev0-backstore
}
trap cleanup EXIT

# Get tests ready
cd /tmp
git clone https://github.com/pjd/pjdfstest.git --depth=1
cd pjdfstest
autoreconf -ifs
./configure
make pjdfstest -j

# Run tests
cd /dkfs
sudo prove -rv /tmp/pjdfstest/tests