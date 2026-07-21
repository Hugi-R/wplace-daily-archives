#!/bin/bash
set -e

# sudo mount -t tmpfs -o size=28g tmpfs /run/media/system/DataBtrfs/wplace/ramfs

for orig_file in /run/media/system/DataBtrfs/wplace/wplace-archives/days2/*.db
do
    file="/run/media/system/DataBtrfs/wplace/ramfs/$(basename $orig_file)"
    cp $orig_file $file

    # Cleanup and index
    sqlite3 $file "DELETE FROM tiles WHERE z != 11;"
    sqlite3 $file "CREATE INDEX IF NOT EXISTS tiles_z_y_x_idx ON tiles (z, y, x);"
    ./target/release/wplace-daily-archives validate $file

    echo "Processing version: 0 from file: $file"
    ./target/release/wplace-daily-archives merge -t 0 $file
    versions=$(sqlite3 $file "SELECT date FROM versions;")
    for v in $versions
    do
        echo "Processing version: $v from file: $file"
        ./target/release/wplace-daily-archives merge -t $v $file
    done

    # Disable WAL mode to allow read-only access to the database file
    sqlite3 $file "PRAGMA journal_mode=DELETE;"

    mv $file $orig_file
    # Free disk space by vacuuming the database (we can't do it on ramfs, vacuum require double the size of the database)
    sqlite3 $orig_file "VACUUM;"
done