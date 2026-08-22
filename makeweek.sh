#!/bin/bash

set -e

WEEKNUM="83"
WEEKBASE="full_2026-08-04T01-36-27Z.db"
WEEKINCREMENTS=(
    "inc_2026-08-06T01-36-50Z.db"
    "inc_2026-08-07T01-42-25Z.db"
    "inc_2026-08-08T01-36-49Z.db"
    "inc_2026-08-09T01-40-00Z.db"
    "inc_2026-08-10T01-38-45Z.db"
    "inc_2026-08-11T01-40-12Z.db"
)

DOWNLOADFOLDER=~/Téléchargements
WORKDIR="/run/media/system/DataBtrfs/wplace/ramfs/${WEEKNUM}" # make it a ramfs for speed, need ~20Go
mkdir -p "$WORKDIR"
echo "Working in ${WORKDIR}"

echo "Making week ${WEEKNUM} from base ${WEEKBASE}"
uvx hf buckets cp "hf://buckets/Hugi-R/wplace-archives/full/${WEEKBASE}" "$DOWNLOADFOLDER"
./target/release/wpda-pipeline makebase --base "$DOWNLOADFOLDER/${WEEKBASE}" --output "${WORKDIR}/w${WEEKNUM}_0.db"
./target/release/wpda-pipeline merge -t 0 "${WORKDIR}/w${WEEKNUM}_0.db"
rm "$DOWNLOADFOLDER/${WEEKBASE}"

for inc in "${WEEKINCREMENTS[@]}"
do
    echo "Ingesting incremental ${inc}"
    uvx hf buckets cp "hf://buckets/Hugi-R/wplace-archives/incremental/${inc}" "$DOWNLOADFOLDER"
    ./target/release/wpda-pipeline ingest --archives "$WORKDIR" --increment "$DOWNLOADFOLDER/${inc}"
    rm "$DOWNLOADFOLDER/${inc}"
done

echo "Result in ${WORKDIR}"
echo "Done"
