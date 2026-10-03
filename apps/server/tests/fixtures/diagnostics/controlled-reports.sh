#!/bin/sh
# Temporary acceptance data: this deliberate assertion marker is not a
# diagnosis of a production defect or permission to change another repository.
case "$1" in
 --symphony-feedback-v2) if [ "$2" = malformed ]; then printf '{malformed'; exit 0; fi; exec python3 -c 'import os,json,pathlib; d=pathlib.Path(os.environ["SYMPHONY_DIAGNOSTIC_DIR"]); report="FAIL-FIRST\n"+"测量 failed\n"*6000+"FAIL-LAST\n"; (d/"report.md").write_text(report); (d/"single.json").write_text(json.dumps({"failures":["FIRST"]+["failure"]*10000+["LAST"]})); print(json.dumps({"protocol_version":2,"check_id":"python","verdict":"fail","fault":None,"artifacts":[{"kind":"report","path":"report.md"},{"kind":"report","path":"single.json"}]}))';;
 timeout) printf 'failure before timeout\n'; sleep 10;;
 crash) printf 'failure before crash\n'; kill -9 $$;;
 malformed) printf '{malformed';;
 flood) yes 'failure overflowing capture';;
 *) printf 'FAIL-FIRST\n'; if [ "$1" = controlled-assertion ]; then printf 'AssertionError: controlled X04 diagnostic reader test datum; no repairable production defect\n'; fi; i=0; while [ "$i" -lt 6000 ]; do printf 'shell failure item %s\n' "$i"; i=$((i+1)); done; printf 'FAIL-LAST\n'; exit 2;;
esac
