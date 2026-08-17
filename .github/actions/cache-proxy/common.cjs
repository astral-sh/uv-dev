const fs = require('node:fs');
const https = require('node:https');
const crypto = require('node:crypto');

const content = Buffer.from('Harmless cache-mode probe. No executable content.\n');
const version = crypto.createHash('sha256').update('uv-cache-proxy-probe-v1').digest('hex');
const prefix = () => `cache-proxy-probe-${process.env.GITHUB_RUN_ID}-${process.env.GITHUB_RUN_ATTEMPT}`;
const output = (name, value) => fs.appendFileSync(process.env.GITHUB_OUTPUT, `${name}=${value}\n`);

function serviceUrl(value) {
  const url = new URL(value);
  if (url.protocol !== 'https:' || !url.hostname.endsWith('.actions.githubusercontent.com') || url.port || url.username || url.password) throw new Error('Unexpected service endpoint');
  return url;
}

function request(url, {method='GET', headers={}, body, address, ca, timeout=15000}={}) {
  return new Promise((resolve, reject) => {
    const options = {method, headers, agent: false, ...(ca ? {ca} : {})};
    if (address) options.lookup = (_host, lookupOptions, callback) => {
      const family=address.includes(':')?6:4;
      if(lookupOptions.all)callback(null,[{address,family}]);else callback(null,address,family);
    };
    const req = https.request(url, options, response => {
      const chunks = [];
      let length = 0;
      response.on('data', chunk => {
        length += chunk.length;
        if (length > 1024 * 1024) req.destroy(Object.assign(new Error('Response too large'), {code:'ETOOBIG'}));
        else chunks.push(chunk);
      });
      response.on('end', () => resolve({status:response.statusCode, headers:response.headers, body:Buffer.concat(chunks)}));
    });
    req.setTimeout(timeout, () => req.destroy(Object.assign(new Error('Request timeout'), {code:'ETIMEDOUT'})));
    req.on('error', error => reject(Object.assign(new Error('Service request failed'), {code:error.code || 'REQUEST_FAILED'})));
    if (body) req.write(body);
    req.end();
  });
}

async function cacheCall(method, data, options={}) {
  const url = serviceUrl(process.env.ACTIONS_RESULTS_URL);
  url.pathname = `/twirp/github.actions.results.api.v1.CacheService/${method}`;
  url.search = '';
  const body = Buffer.from(JSON.stringify(data));
  const response = await request(url, {...options, method:'POST', headers:{Authorization:`Bearer ${process.env.ACTIONS_RUNTIME_TOKEN}`, 'Content-Type':'application/json', 'Content-Length':String(body.length)}, body});
  let parsed;
  try { parsed = JSON.parse(response.body); } catch { throw new Error(`Cache response was not JSON (${response.status})`); }
  return {...response, data:parsed};
}

const readMetadata = (key, options) => cacheCall('GetCacheEntryDownloadURL', {key, version, restore_keys:[]}, options);
const isDenied = (response, operation) => response.status === 403 && response.headers['x-uv-cache-proxy'] === 'denied' && response.data.msg?.startsWith(`cache ${operation} denied:`);

async function storage(value, options={}) {
  const url = new URL(value);
  if (url.protocol !== 'https:' || !url.hostname.endsWith('.blob.core.windows.net')) throw new Error('Unexpected storage endpoint');
  try { return await fetch(url, {...options, redirect:'error', signal:AbortSignal.timeout(30000)}); }
  catch { throw new Error('Disposable storage request failed'); }
}

async function writeCache(key) {
  const created = await cacheCall('CreateCacheEntry', {key,version});
  const url = created.data.signed_upload_url || created.data.signedUploadUrl;
  if (isDenied(created,'write')) return {http:created.status,denied:true};
  if (!created.data.ok || !url) return {http:created.status,denied:false,finalized:false};
  const uploaded = await storage(url, {method:'PUT',headers:{'x-ms-blob-type':'BlockBlob','x-ms-version':'2023-11-03','Content-Type':'application/octet-stream'},body:content});
  if (!uploaded.ok) throw new Error('Disposable cache upload failed');
  const finalized = await cacheCall('FinalizeCacheEntryUpload', {key,version,size_bytes:String(content.length)});
  return {http:created.status,denied:false,finalized:finalized.data.ok===true,cacheId:finalized.data.entry_id || finalized.data.entryId || null};
}

function emit(label, result) {
  const rendered=JSON.stringify({label,...result});
  console.log(`CACHE_PROXY_RESULT=${rendered}`);
  if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, `\n\`\`\`json\n${JSON.stringify({label,...result},null,2)}\n\`\`\`\n`);
}

module.exports={content,version,prefix,output,serviceUrl,request,cacheCall,readMetadata,isDenied,storage,writeCache,emit};
