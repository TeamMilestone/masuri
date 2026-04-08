#!/bin/bash
# Test masuri accuracy against ground truth
# Returns: correct wrong missing total
cd "$(dirname "$0")/.."

MASURI="./rszbar/target/release/masuri"
correct=0; wrong=0; missing=0; total=0
wrong_list=""
missing_list=""

while IFS=$'\t' read -r id expected; do
    fname=$(ls samples/${id}_barcode_${expected}.jpg 2>/dev/null | head -1)
    if [ -z "$fname" ]; then continue; fi
    total=$((total+1))
    result=$($MASURI "$fname" 2>/dev/null | grep CODE-128 | head -1 | sed 's/.*: //' | sed 's/ \[CODE-128\].*//')
    if [ -z "$result" ]; then
        missing=$((missing+1))
        missing_list="$missing_list #$id"
    elif [ "$result" = "$expected" ]; then
        correct=$((correct+1))
    else
        wrong=$((wrong+1))
        wrong_list="$wrong_list #$id($result!=$expected)"
    fi
done << 'TRUTH'
001	44115800374
002	6129109005315
003	697301115150
004	301059717470
005	301059716221
006	301059814593
007	301059801702
008	301059785856
009	301059692572
010	301059821453
011	301059801153
012	301059804163
013	301059798950
014	301059654050
015	301059802450
016	301059769616
017	301059804583
018	301059683472
019	302445013712
020	530777556931
021	301059645366
022	301059628382
023	301059678443
024	301059655450
025	301059810673
026	302444947853
027	521542238913
028	530778777930
029	302444774846
030	521542238913
031	301059724901
032	302444839051
034	301059879203
035	301059784913
037	303101218735
038	302444793750
039	530778611772
040	301059764134
041	301059764134
042	301059664535
043	301059664535
044	301059761743
045	301059761743
046	301059761743
047	301059743064
048	301059753785
049	6892097819727
050	301059804480
051	301059820554
052	301059674755
053	301059670146
054	301059857116
055	301059748056
056	301059651073
057	301059817286
058	301059756935
059	301059678561
060	304393331563
061	697118391103
062	301059643900
064	301059800405
065	301059686891
068	540773132706
TRUTH

pct=$(python3 -c "print(f'{$correct/$total*100:.1f}')" 2>/dev/null || echo "0")
echo "$correct/$total ($pct%) | wrong=$wrong missing=$missing"
if [ -n "$wrong_list" ]; then echo "  WRONG:$wrong_list"; fi
if [ -n "$missing_list" ]; then echo "  MISSING:$missing_list"; fi
