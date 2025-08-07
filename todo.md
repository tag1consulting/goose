# In metrics.rs
## Fix TODO
```	
             let started = Local
                .timestamp_opt(step[0].timestamp.timestamp(), 0)
                // @TODO: error handling
                .unwrap()
                .format("%y-%m-%d %H:%M:%S");
```

## Fix TODO:
```
            let stopped = Local
                .timestamp_opt(step[1].timestamp.timestamp(), 0)
                // @TODO: error handling
                .unwrap()
                .format("%y-%m-%d %H:%M:%S");
```
