#!/usr/bin/env python3
"""Validate a requirements bundle, not a product implementation.
Usage: python scripts/validate_bundle.py [--report 09-audit/check-results.json]
Requires Python >=3.10 and jsonschema==4.26.0. No network or credentials used.
"""
from __future__ import annotations
import argparse, csv, hashlib, itertools, json, math, re, subprocess, sys
from datetime import datetime, timezone
from pathlib import Path
try:
    from jsonschema import Draft202012Validator, FormatChecker
except ImportError:
    raise SystemExit('Missing jsonschema. Install scripts/requirements-validation.txt in an isolated environment; no checks were run.')
ROOT=Path(__file__).resolve().parents[1]
CHECKS=[]

def load(path): return json.loads((ROOT/path).read_text(encoding='utf-8'))
def digest(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def check(name, condition, detail=''):
    CHECKS.append({'check':name,'status':'PASS' if bool(condition) else 'FAIL','detail':str(detail)})
def walk(value, path=''):
    if isinstance(value,dict):
        yield path,value
        for key,item in value.items(): yield from walk(item,path+'/'+str(key))
    elif isinstance(value,list):
        for index,item in enumerate(value): yield from walk(item,path+'/'+str(index))
def resolve(doc, ref):
    if not ref.startswith('#/'): raise ValueError('Nonlocal reference: '+ref)
    cur=doc
    for part in ref[2:].split('/'):
        cur=cur[part.replace('~1','/').replace('~0','~')]
    return cur

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=str)
    args=parser.parse_args()
    meta=load('bundle-metadata.json');cat=load('01-product/requirement-catalog.json')
    req={x['id']:x for x in cat['requirements']};ac={x['id']:x for x in cat['criteria']};nfr={x['id']:x for x in cat['quality']};features={x['id']:x for x in cat['features']}
    cases=load('06-testing/test-cases.json')['cases'];case_by_id={x['id']:x for x in cases}
    suites=load('06-testing/suites.json')['suites'];suite_by_id={x['id']:x for x in suites}
    for name,objects,key in [('features',features,'features'),('requirements',req,'requirements'),('acceptance criteria',ac,'acceptance_criteria'),('NFR',nfr,'nfrs'),('test cases',case_by_id,'test_cases_designed'),('test suites',suite_by_id,'test_suites_designed')]:
        check('metadata count '+name,len(objects)==meta[key],len(objects))
    for prefix,ids in [('F',features),('REQ',req),('AC',ac),('NFR',nfr)]:
        check('dense initial IDs '+prefix,set(ids)=={f'{prefix}-{i:03}' for i in range(1,len(ids)+1)})
    check('unique test IDs',len(case_by_id)==len(cases))
    check('unique suite IDs',len(suite_by_id)==len(suites))
    check('product tests honestly not run',meta['product_tests_run']==0 and all(x['execution_status']=='NOT_RUN' for x in cases))
    check('independent review not fabricated',meta['independent_review']=='PENDING')
    check('tracker not activated',meta['tracking']=='NOT_INITIALIZED')
    for f in features.values():
        check('feature ownership '+f['id'],bool(f['requirement_ids']) and all(r in req and req[r]['feature_id']==f['id'] for r in f['requirement_ids']))
    for rid,r in req.items():
        mapped=[a for a in ac.values() if a['requirement_id']==rid]
        check('REQ->AC '+rid,bool(mapped) and all(a['feature_id']==r['feature_id'] for a in mapped))
        check('single obligation '+rid,len(re.findall(r'\bshall\b',r['text'],re.I))==1 and len(r['text'].split())<=45)
    for cid,c in {**ac,**nfr}.items():
        primary=[t for t in cases if t.get('primary_criterion')==cid]
        check('primary designed case '+cid,bool(primary),','.join(t['id'] for t in primary))
    for t in cases:
        sid=t['suite_id']
        check('test suite backlink '+t['id'],sid in suite_by_id and t['id'] in suite_by_id[sid]['case_ids'])
        check('test observable fields '+t['id'],bool(t.get('preconditions')) and bool(t.get('steps')) and bool(t.get('expected')) and bool(t.get('evidence_required')))
        check('test requirement link '+t['id'],all(x in req for x in t['requirement_ids']))
    for sid,suite in suite_by_id.items():
        check('suite nonempty '+sid,bool(suite['case_ids']) and all(x in case_by_id and case_by_id[x]['suite_id']==sid for x in suite['case_ids']))
    with (ROOT/'06-testing/traceability.csv').open(encoding='utf-8-sig',newline='') as f:trace=list(csv.DictReader(f))
    check('trace all cases',{x['test_case'] for x in trace}==set(case_by_id))
    check('trace all criteria',{x['criterion'] for x in trace}==set(ac)|set(nfr))
    check('trace links',all(x['test_case'] in case_by_id and x['suite'] in suite_by_id and x['execution_status']=='NOT_RUN' for x in trace))
    for part in load('07-delivery/release-partition.json')['partitions']:
        check('phase requirements '+part['id'],set(part['requirement_ids'])=={r['id'] for r in req.values() if r['phase']==part['id']})
        check('phase criteria '+part['id'],set(part['criterion_ids'])=={a['id'] for a in ac.values() if a['phase']==part['id']})
    # Unmodified validator and its supplied positive/negative fixture suite.
    val=subprocess.run([sys.executable,str(ROOT/'vendor/requirements-spec/scripts/validate_spec.py'),str(ROOT/meta['srs_path'])],capture_output=True,text=True)
    check('upstream SRS validator',val.returncode==0 and '0 error(s), 0 warning(s)' in val.stdout,val.stdout.strip())
    test=subprocess.run(['sh',str(ROOT/'vendor/requirements-spec/tests/run.sh')],capture_output=True,text=True)
    check('upstream validator 15-class selftest',test.returncode==0,(test.stdout+test.stderr).strip())
    # JSON syntax, schema validity, local references and positive/negative wire vectors.
    json_files=[p for p in ROOT.rglob('*.json') if 'vendor' not in p.parts and '09-audit' not in p.parts]
    for p in json_files:
        try:json.loads(p.read_text(encoding='utf-8'));ok=True;msg=''
        except Exception as e:ok=False;msg=str(e)
        check('JSON parse '+str(p.relative_to(ROOT)),ok,msg)
    wire=load('03-contracts/contracts.json');shared=load('03-contracts/shared-contracts.json');examples=load('06-testing/fixtures/wire-examples.json')['examples']
    check('schema count',len(wire['$defs'])==meta['schemas'])
    check('shared definitions identical',all(wire['$defs'].get(k)==v for k,v in shared['$defs'].items()))
    check('wire example coverage',set(examples)==set(wire['$defs']))
    negative_count=0
    for name,schema in wire['$defs'].items():
        try:Draft202012Validator.check_schema(schema);ok=True;msg=''
        except Exception as e:ok=False;msg=str(e)
        check('schema valid '+name,ok,msg)
        local={'$schema':wire['$schema'],'$defs':wire['$defs'],'$ref':'#/$defs/'+name}
        v=Draft202012Validator(local,format_checker=FormatChecker())
        errors=list(v.iter_errors(examples[name]))
        check('wire example '+name,not errors,'; '.join(e.message for e in errors[:3]))
        if schema.get('type')=='object':
            for field in schema.get('required',[]):
                bad=dict(examples[name]);bad.pop(field,None);negative_count+=1
                check('missing required rejected '+name+'.'+field,not v.is_valid(bad))
            if schema.get('additionalProperties') is False:
                bad=dict(examples[name]);bad['unexpected_test_field']=True;negative_count+=1
                check('unknown field rejected '+name,not v.is_valid(bad))
    for fname in ['03-contracts/contracts.json','03-contracts/shared-contracts.json','03-contracts/openapi.json','03-contracts/masonwing-platform.openapi.json']:
        doc=load(fname)
        refs=[(p,x['$ref']) for p,x in walk(doc) if '$ref' in x]
        for path,reference in refs:
            try:resolve(doc,reference);ok=True;msg=''
            except Exception as e:ok=False;msg=str(e)
            check('resolved reference '+fname+path,ok,msg)
    # This is bounded OpenAPI structure/local-ref verification, not a full external OAS validator.
    for fname in ['03-contracts/openapi.json','03-contracts/masonwing-platform.openapi.json']:
        doc=load(fname);ids=[]
        check('OpenAPI version '+fname,doc.get('openapi')=='3.1.0')
        for path,item in doc['paths'].items():
            for method,op in item.items():
                if method not in {'get','put','post','patch','delete','options','head'}:continue
                ids.append(op['operationId'])
                parameters=item.get('parameters',[])+op.get('parameters',[])
                declared={p['name'] for p in parameters if p.get('in')=='path' and p.get('required') is True}
                check('OpenAPI path params '+method+' '+path,set(re.findall(r'\{([^}]+)\}',path))==declared)
                check('OpenAPI response '+method+' '+path,bool(op.get('responses')))
        check('OpenAPI unique operations '+fname,len(ids)==len(set(ids)))
    ops=load('03-contracts/operations.json')['operations'];oas=load('03-contracts/openapi.json');platform=load('03-contracts/masonwing-platform.openapi.json')
    check('typed operation count',len(ops)==meta['api_operations'])
    for op in ops:
        prefix='marketing/' if meta['product']=='GLEANBIRD' else ''
        path='/v1/tenants/{tenant_id}/commands/'+prefix+op['operation']
        check('typed operation published '+op['operation'],path in oas['paths'] and op['request_schema'] in wire['$defs'])
    for endpoint in load('03-contracts/data-plane.json')['endpoints']:
        check('data plane published '+endpoint['path'],endpoint['path'] in platform['paths'] and endpoint['method'].lower() in platform['paths'][endpoint['path']])
    cross=load('03-contracts/cross-product-map.json')
    if meta['product']=='GLEANBIRD':
        check('crosspack all marketing features covered',{x['consumer_feature'].split(':',1)[1] for x in cross['links']}==set(features))
        check('crosspack all marketing requirements covered',{r.split(':',1)[1] for x in cross['links'] for r in x['consumer_requirements']}==set(req))
    else:
        check('crosspack provider requirement IDs resolve',all(r.split(':',1)[1] in req for x in cross['links'] for r in x['provider_requirements']))
    privileged={'catalog.review','release.qualify','identity.configure','policy.propose','plugin.revoke'}
    if meta['product']=='MASONWING':
        check('safety-critical platform actions not tenant Owner',all('OWNER' not in x['roles_candidate'] and bool(set(x['roles_candidate']) & {'PLATFORM_OPERATOR','SECURITY_REVIEWER'}) for x in ops if x['operation'] in privileged))
    lock=load('03-contracts/contract-lock.json')
    check('contract digest pinned',digest(ROOT/'03-contracts/shared-contracts.json')==lock['sha256'])
    # Finite transition oracle checks. These do NOT run product transition handlers.
    machines=load('03-contracts/state-machines.json');vectors=load('06-testing/state-vectors.json')['vectors']
    for name,m in machines.items():
        check('state initial '+name,m['initial'] in m['states'])
        edges={(a,b):event for a,b,event in m['edges']}
        check('state edges valid '+name,len(edges)==len(m['edges']) and all(a in m['states'] and b in m['states'] for a,b in edges))
        expected={(a,b) for a in m['states'] for b in m['states'] if a!=b};actual=[v for v in vectors if v['machine']==name]
        check('finite all distinct state pairs '+name,{(v['from'],v['to']) for v in actual}==expected and len(actual)==len(expected))
        check('transition oracle '+name,all(v['allowed_adjacency']==((v['from'],v['to']) in edges) for v in actual))
    pw=load('06-testing/pairwise-vectors.json');factors=pw['factors'];rows=pw['vectors'];universe=set()
    for a,b in itertools.combinations(factors,2):
        universe.update((a,av,b,bv) for av in factors[a] for bv in factors[b])
    seen={(a,r['factors'][a],b,r['factors'][b]) for r in rows for a,b in itertools.combinations(factors,2)}
    check('all declared factor pairs represented',seen==universe,f'{len(seen)}/{len(universe)} pairs; {len(rows)} vectors')
    for row in rows:
        d=row['factors'];allow=d['role']!='VIEWER' and d['tenant']=='SAME' and d['credential']=='VALID' and d['policy']=='ALLOW' and d['source_rights']=='VALID' and (d['role']!='SERVICE' or d['grant']=='ACTIVE')
        check('pairwise reference decision '+row['id'],row['expected_decision']==('ALLOW' if allow else 'DENY'))
    rv=load('06-testing/reference-vectors.json')
    for i,v in enumerate(rv['measurement']):
        value=v['mentions']/v['eligible'] if v['eligible'] else None
        check('metric denominator reference '+str(i),value==v['rate'] and v['planned']==v['eligible']+v['failed']+v['ineligible'])
    s=rv['score'];calculated=max(0,min(100,sum(a*b for a,b in zip(s['weights'],s['factors']))-s['risk_penalty']))
    check('score arithmetic reference',math.isclose(calculated,s['score']) and math.isclose(sum(s['weights']),1))
    check('unknown metric reference',s['missing_factor_score'] is None and rv['measurement'][1]['rate'] is None)
    check('known cost arithmetic reference',rv['budget'][1]['reserved']-rv['budget'][1]['usage']==rv['budget'][1]['released'])
    check('unknown cost retains reservation reference',rv['budget'][2]['state']=='UNKNOWN' and rv['budget'][2]['released']==0)
    works=load('07-delivery/work-packages.json')['packages'];workmap={x['id']:x for x in works}
    visiting=set();visited=set()
    def dfs(w):
        if w in visiting:raise ValueError('Cycle '+w)
        if w in visited:return
        if w not in workmap:raise ValueError('Missing dependency '+w)
        visiting.add(w)
        for dep in workmap[w]['dependency_ids']:dfs(dep)
        visiting.remove(w);visited.add(w)
    try:
        for w in workmap:dfs(w)
        ok=True;msg=''
    except Exception as e:ok=False;msg=str(e)
    check('work package DAG',ok,msg)
    check('work package requirement coverage',{r for w in works for r in w['requirement_ids']}==set(req))
    check('work package criterion coverage',{a for w in works for a in w['criterion_ids']}==set(ac))
    for source in load('08-governance/source-manifest.json')['sources']:
        check('input hash '+source['id'],digest(ROOT/source['path'])==source['sha256'])
    assumptions=load('08-governance/assumptions.json');questions=load('08-governance/open-questions.json')
    # These may be an array or an envelope to preserve authoring format compatibility.
    aa=assumptions if isinstance(assumptions,list) else assumptions.get('assumptions',[])
    qq=questions if isinstance(questions,list) else questions.get('questions',questions.get('open_questions',[]))
    check('assumption owners',bool(aa) and all(x.get('owner') for x in aa))
    check('question owners',len(qq)==meta['open_questions'] and all(x.get('owner') for x in qq))
    if (ROOT/'MANIFEST.sha256').exists():
        for line in (ROOT/'MANIFEST.sha256').read_text().splitlines():
            expected,path=line.split('  ',1)
            check('file digest '+path,(ROOT/path).is_file() and digest(ROOT/path)==expected)
    failures=[c for c in CHECKS if c['status']=='FAIL']
    report={'product':meta['product'],'checked_at':datetime.now(timezone.utc).isoformat(timespec='seconds'),'scope':'DOCUMENT_SCHEMA_REFERENCE_CHECKS_ONLY','result':'PASS' if not failures else 'FAIL','checks_executed':len(CHECKS),'checks_passed':len(CHECKS)-len(failures),'checks_failed':len(failures),'schema_negative_vectors_executed':negative_count,'requirement_to_ac_coverage':len({a['requirement_id'] for a in ac.values()})/len(req),'criteria_to_primary_case_coverage':len({t['primary_criterion'] for t in cases if t.get('primary_criterion')})/(len(ac)+len(nfr)),'product_tests_executed':0,'full_external_openapi_validator':'NOT_RUN; optional dependency unavailable in authoring runtime. OpenAPI structure, local refs, typed schemas and path parameters were checked locally.','limitations':['State/pairwise/metric checks exercise the specified reference model, not an implementation.','No Rust build, Wasmtime execution, browser UI, live provider, CMS, Keycloak deployment, load or DR test was performed.','Independent review and production policy approval remain pending.'],'checks':CHECKS}
    if args.report:
        out=(ROOT/args.report).resolve()
        if ROOT not in out.parents:raise SystemExit('Report path must stay inside bundle.')
        out.parent.mkdir(parents=True,exist_ok=True);out.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(f"{meta['product']}: {report['result']}; {len(CHECKS)} document checks; {len(failures)} failures; product tests=0")
    for f in failures[:30]:print('FAIL:',f['check'],f['detail'][:500])
    return 1 if failures else 0
if __name__=='__main__':
    raise SystemExit(main())
