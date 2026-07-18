for file in /run/media/system/DataBtrfs/wplace/wplace-archives/days2/*.db
do
    echo "Processing version: 0 from file: $file"
    ./target/release/wplace-daily-archives merge -t 0 $file
    versions=$(sqlite3 $file "SELECT date FROM versions;")
    for v in $versions
    do
        echo "Processing version: $v from file: $file"
        ./target/release/wplace-daily-archives merge -t $v $file
    done
done