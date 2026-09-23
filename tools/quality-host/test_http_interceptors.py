"""Installed collector import support must not omit intercepted HTTP calls."""
import subprocess
import unittest
from http_contract import PLUGIN


class InterceptorInventoryTests(unittest.TestCase):
    def test_metadata_imports_preserve_inventory_and_rejections(self):
        script = r"""
const assert=require('assert/strict'),C=require(process.argv[1]+'/clients.cjs');
const config="import {provideHttpClient,withInterceptors} from '@angular/common/http';provideHttpClient(withInterceptors([auth]));";
const auth="import {HttpClient,HttpInterceptorFn} from '@angular/common/http';const auth:HttpInterceptorFn=(req,next)=>{const http=inject(HttpClient);http.get<AuthResponse>('/api/auth/session');return next(req.clone({setHeaders:{csrf:'test'}}));};";
assert.deepEqual(C.inventory({'config.ts':config,'auth.ts':auth}),[
 {file:'auth.ts',method:'GET',path:'/api/auth/session',typeName:'AuthResponse'}]);
for(const body of [auth.replace('return next(',"fetch('/api/hidden');return next("),auth.replace('http.get','http["get"]')])
 assert.throws(()=>C.inventory({'auth.ts':body}));
for(const imports of ['HttpInterceptorFn as Alias','withInterceptors as Alias','HttpBackend'])
 assert.throws(()=>C.inventory({'bad.ts':"import {"+imports+"} from '@angular/common/http';"}));
assert.deepEqual(C.inventory({'type.ts':"import type {HttpInterceptorFn} from '@angular/common/http';"}),[]);
"""
        result = subprocess.run(['node', '-e', script, str(PLUGIN)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == '__main__': unittest.main(verbosity=2)
