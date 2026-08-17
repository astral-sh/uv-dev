if(process.env.UV_CACHE_PROXY_ACTIVE!=='1')throw new Error('Cache proxy pre hook did not complete');
console.log('Cache proxy is active.');
